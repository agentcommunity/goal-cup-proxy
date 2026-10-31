#!/bin/bash
# Merge queue, driven by whoever waits on it (lane waiter agents), for at most $1 seconds (default 540).
# Request: /root/gc-night/mrq/<id>.req, JSON {"branch", "message", "refresh_lock", "first"}.
# Result:  <id>.res, JSON {"exit", "output", "gate_log"}; exit 0 merged, 1 gate FAIL, 3 conflict, 4 push refused.
# A request is claimed by an atomic rename and its merge runs detached (setsid), so a waiter that stops
# never loses a result. merge.sh holds the integration lock, so merges never overlap.
# Requests wait until PRD-14's gate change is on integration, except one marked "first".
Q=/root/gc-night/mrq; REPO=/home/user/goal-cup-proxy
end=$(( $(date +%s) + ${1:-540} ))
# A claim with no result after 90 minutes goes back to the queue.
for c in "$Q"/*.claimed; do
  [ -e "$c" ] || continue
  id=$(basename "$c" .claimed)
  [ -e "$Q/$id.res" ] && { mv "$c" "$Q/done/$id.req"; continue; }
  [ -n "$(find "$c" -mmin +90)" ] && mv "$c" "$Q/$id.req"
done
while [ "$(date +%s)" -lt "$end" ]; do
  git -C "$REPO" fetch -q origin 2>/dev/null
  gate_in=0; git -C "$REPO" merge-base --is-ancestor origin/v1.0/prd-14-gate origin/v1.0/integration 2>/dev/null && gate_in=1
  for r in $(ls -tr "$Q"/*.req 2>/dev/null); do
    id=$(basename "$r" .req)
    IFS=$'\t' read -r br msg rl first < <(python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print("\t".join([d["branch"], d.get("message","merge").replace("\t"," ").replace("\n"," "), str(int(bool(d.get("refresh_lock")))), str(int(bool(d.get("first"))))]))' "$r") || { mv "$r" "$Q/done/"; continue; }
    [ "$gate_in" = 1 ] || [ "$first" = 1 ] || continue
    mv "$r" "$Q/$id.claimed" 2>/dev/null || continue
    flags=""; [ "$rl" = 1 ] && flags="--refresh-lock"
    setsid nohup bash -c '
      out=$(/root/gc-tools/merge.sh "$1" "$2" $3 2>&1); code=$?
      gl=$(ls -t /root/gc-night/gate-logs/*.log 2>/dev/null | head -1)
      python3 -c "import json,sys;json.dump({\"exit\":int(sys.argv[2]),\"output\":sys.argv[3][-3000:],\"gate_log\":sys.argv[4]},open(sys.argv[1]+\".tmp\",\"w\"))" "$4/$5.res" "$code" "$out" "$gl"
      mv "$4/$5.res.tmp" "$4/$5.res"; mv "$4/$5.claimed" "$4/done/$5.req"
    ' _ "$br" "$msg" "$flags" "$Q" "$id" >/dev/null 2>&1 < /dev/null &
  done
  sleep 20
done
