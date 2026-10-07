#!/bin/sh
# The shape M0 §2.5 measured on Codex: an npm wrapper whose own pid is NOT the process
# holding the session. The wrapper stays in the foreground; the process that matters is a
# child it forked a moment later, and `kill -9` on the wrapper leaves that child running
# with the thread-writer lock still held (D22).
#
# Argument 1 is a path this script writes the grandchild's pid to. It is written to a
# neighbouring temporary name and renamed, because a reader that waits for the file to
# EXIST would otherwise be able to read it while it is still empty.
set -e
pidfile="$1"
sleep 30 &
child=$!
printf '%s' "$child" > "$pidfile.partial"
mv "$pidfile.partial" "$pidfile"
wait "$child"
