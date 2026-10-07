#!/bin/sh
# Build an orphan before the seal-time Darwin watcher exists, then make the registered root
# look like a clean singleton. `wait $!` is the test's barrier: it returns only after the
# intermediate subshell has written the pid file and exited, so the subshell is reaped here
# and the root that then execs into `sleep` inherits no unreaped zombie child (a zombie
# would keep the root's ppid walk at [root, <defunct>] for good). The orphan has already
# left this root's ppid walk by then, and the test waits for that before calling terminate.
( sleep 300 & echo $! >"$1" ) &
wait $!
exec sleep 300
