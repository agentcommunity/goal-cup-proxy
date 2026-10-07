#!/bin/sh
# The shape M0-FINDINGS §2.5 measured: a launcher that is not the session. The real work
# runs in a child, so killing the pid the tracker spawned leaves the child holding the
# writer lock, reparented to pid 1. The tracker must discover this child, not assume it.
sleep 300 &
wait
