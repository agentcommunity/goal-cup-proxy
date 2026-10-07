#!/bin/sh
# A tree that keeps forking while it is being terminated. Each generation ignores SIGTERM,
# starts a worker, pauses, and only then starts the next generation, so a new descendant
# appears after the walk that found its parent — repeatedly, not once.
#
# Bounded at three generations on purpose. What this pins is a moving target and a
# termination that has to stay bounded while chasing it; a fork bomb would pin neither.
trap '' TERM
depth="${1:-0}"
sleep 300 &
if [ "$depth" -lt 3 ]; then
    sleep 0.3
    "$0" "$((depth + 1))" &
fi
wait
