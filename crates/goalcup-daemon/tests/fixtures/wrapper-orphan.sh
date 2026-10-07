#!/bin/sh
# A grandchild that leaves the ppid WALK without leaving the process GROUP.
#
# The subshell backgrounds a sleeper and exits at once without waiting for it, so the
# sleeper reparents to pid 1: `descendants()` is ppid-only, so from that moment nothing
# rooted at this leader can reach it. It stays in this leader's process group, which is
# what makes it reachable by `killpg` and by nothing else — the L3-I4 case.
#
# $1 is the file the orphan's pid is written to, so the test names the process that must
# not survive rather than guessing at it.
( sleep 300 & echo $! >"$1" ) &
# Always inside `wait`, so this leader is signallable and does not run off the end of the
# script before the test has a tree to terminate.
while :; do
    sleep 300 &
    wait
done
