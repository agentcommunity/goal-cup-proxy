#!/bin/sh
# The easy case, kept because it is the one that must not regress: a launcher that execs
# into the session keeps the pid, so the tree is one process and the descendant walk
# finds nothing below it. A `terminate` that only ever kills discovered children would
# silently do nothing here.
exec sleep 300
