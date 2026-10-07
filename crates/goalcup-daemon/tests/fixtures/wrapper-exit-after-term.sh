#!/bin/sh
# Starts a native child from its TERM handler and exits immediately. By the next termination
# poll, the registered shell is gone and its former PID may be recycled; the new child is
# deliberately still alive so a bare PID/PGID retry would have an observable victim.
out="$1"
trap 'sleep 300 & echo $! >"$out"; exit 0' TERM
while :; do
    sleep 300 &
    wait
done
