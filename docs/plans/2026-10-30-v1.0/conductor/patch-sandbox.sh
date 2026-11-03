#!/usr/bin/env bash
# Make the sandbox SDK honour HTTPS_PROXY (ESM build, @vercel/sandbox 3.9.1).
# Usage: patch-sandbox.sh NODE_MODULES
# Exit 0 (prints "patched" or "already patched"), 2 usage, 5 unknown SDK layout.
set -u
nm=${1:-}
[ -n "$nm" ] && [ -d "$nm" ] || { echo "usage: patch-sandbox.sh NODE_MODULES" >&2; exit 2; }
f=$nm/@vercel/sandbox/dist/api-client/base-client.js
old='import { Agent } from "undici";'
new='import { EnvHttpProxyAgent as Agent } from "undici";'
if [ -f "$f" ] && grep -qxF "$new" "$f"; then echo "already patched"; exit 0; fi
if [ -f "$f" ] && grep -qxF "$old" "$f"; then
  tmp=$(mktemp) || exit 1
  while IFS= read -r line || [ -n "$line" ]; do
    if [ "$line" = "$old" ]; then printf '%s\n' "$new"; else printf '%s\n' "$line"; fi
  done < "$f" > "$tmp" && cat "$tmp" > "$f"
  rm -f "$tmp"
  echo "patched"; exit 0
fi
echo "unknown @vercel/sandbox layout: $f" >&2
exit 5
