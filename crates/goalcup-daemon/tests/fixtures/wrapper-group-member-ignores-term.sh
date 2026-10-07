#!/bin/sh
# A leader that owns its process group (the harness always spawns a fixture via
# `Command::process_group(0)`, so this leader already leads a fresh group of its own — see
# `spawn_fixture`'s `own_group`) traps TERM and exits quickly. Before that, it has already
# backgrounded a subshell that itself backgrounds a TERM-ignoring member and exits at once
# without waiting — so the member reparents to pid 1 almost immediately, while staying in
# the leader's process group, because `setpgid` is never called on it: only an explicit
# `setpgid` (or a session change) moves a process out of the group it was forked into,
# reparenting alone never does.
#
#   this script (leader, own group, registered root)
#     ( subshell, exits immediately )       -- reparented away, out of the ppid walk
#       member (`trap '' TERM`, sleeping)   -- reparented to pid 1, STILL in the leader's
#                                              group
#
# `killpg` on the leader's group therefore still reaches the member (group membership
# survives the reparent); the member simply ignores the SIGTERM it delivers. The point this
# fixture pins is what the report claims while that member is still alive, not whether the
# signal was sent.
#
# $1 = the file the member's pid is written to, so the test names the process that must not
#      survive rather than guessing at it.
out="$1"
( trap '' TERM; sleep 300 & echo $! >"$out" ) &
trap 'sleep 0.05; exit 0' TERM
# Always inside `wait`, as the other fixtures here are: a shell blocked on a foreground
# command defers a trapped signal until that command returns, and a shell that ran off the
# end of its script would have exited before the signal ever arrived.
while :; do
    sleep 300 &
    wait
done
