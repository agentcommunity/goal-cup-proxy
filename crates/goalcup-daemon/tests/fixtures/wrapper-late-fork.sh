#!/bin/sh
# The M0-FINDINGS §2.5 shape, one turn later: a wrapper that starts a FRESH session from
# its own SIGTERM handler and only then leaves. The new child is forked while the wrapper
# is still alive — so a walk taken during the shutdown can still see it — and is reparented
# to pid 1 the moment the wrapper exits, so a walk taken afterwards never can. A
# termination that enumerates the tree once and then trusts that list reports this tree as
# gone while it is still running.
#
# $1 is the file the new child's pid is written to, so the test can name the process that
# must not survive rather than guess at it.
out="$1"
trap 'sleep 300 & echo $! >"$out"; sleep 1; exit 0' TERM
# Always inside `wait`, which a trapped signal interrupts — a shell blocked in a FOREGROUND
# command defers its handler until that command returns, and a shell that ran off the end
# of its script would have exited before the signal arrived at all.
while :; do
    sleep 300 &
    wait
done
