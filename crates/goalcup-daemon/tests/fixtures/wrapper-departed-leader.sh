#!/bin/sh
# A leader that EXITS, leaving a member behind in the process group it created.
#
# The sleeper is backgrounded and its pid written out, and then this leader returns at
# once: the sleeper reparents to pid 1 and stays in this leader's process group, so the
# group is still alive while the number that leads it is free for the OS to hand to
# anybody. That is the ordinary seal — the participant quit hours ago — and it is the shape
# a `killpg` on the registered pgid signals a stranger's tree in.
#
# $1 is the file the surviving member's pid is written to, so the test names the process
# that must NOT be signalled rather than guessing at it.
sleep 300 &
echo $! >"$1"
exit 0
