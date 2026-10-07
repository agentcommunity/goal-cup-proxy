#!/bin/sh
# goal-cup capture shim for Codex hooks.
# Codex hook handlers are "command" or "mcp_tool" only -- there is no http handler type
# (learn.chatgpt.com/docs/hooks: "command and mcp_tool handlers are supported.
#  prompt and agent handlers are parsed but skipped."), so this shim forwards raw bytes.
#
# argv[1] = hook event name, used ONLY as the URL path.
# stdin   = the hook payload, forwarded byte-for-byte, never parsed.
# stdout  = the receiver's response body, verbatim, so a blocking receiver response
#           ({"decision":"block","reason":...}) reaches Codex unchanged.
# Fails open: any curl error yields empty stdout and exit 0.
EVENT="$1"
BASE="${GOALCUP_RECV:-http://127.0.0.1:7777}"
curl -s -S --connect-timeout 1 --max-time 3 \
     -X POST \
     -H 'Content-Type: application/json' \
     -H "X-Goalcup-Event: ${EVENT}" \
     --data-binary @- \
     "${BASE}/${EVENT}" 2>/dev/null || true
exit 0
