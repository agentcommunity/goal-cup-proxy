#!/bin/sh
# The shape the REAL Codex tree has, measured in a live session on 2026-09-15:
#
#   goalcup start            pgid 61050   the registered root
#     node .../bin/codex     pgid 61050   the npm wrapper
#       codex (native)       pgid 61050   the process holding the session
#         codex-code-mode-host  pgid 62390   ** its own process group **
#
# The helper calls `setpgid` and leaves the launch's process group while REMAINING a
# descendant. That combination is what this fixture exists to pin: `killpg` on the group
# the launch leads never reaches it, and the ppid walk is the only thing that does. A
# termination that had quietly become group-only would pass every other fixture here and
# leave this process running.
#
# `set -m` used to be how this fixture made that same syscall: with job control enabled,
# each background job starts in a new process group. It was vacuous on Linux, where
# `/bin/sh` is dash, which turns job control off the moment it detects no controlling tty --
# the helper never actually left the group, and only this fixture's own vacuity guard
# (`assert_ne!(pgid_of(escaped), pgid_of(root), ...)` in `process_kill.rs`, kept exactly as
# it was) caught it. macOS has no `setsid(1)` either. `tests/support/tree.rs` makes the same
# call in Rust instead (`Command::process_group(0)`, portable to both), re-entering this
# compiled test binary itself under a hard-linked `codex` name -- see that file's module doc.
#
# $1 = the hard-linked "codex" helper `tests/support/tree.rs` built for this test.
# $2 = the file the escaped grandchild's pid is written to, so the test names the process
#      that must not survive rather than guessing at it.
helper="$1"
out="$2"
GOALCUP_FIXTURE_ROLE=native-interior GOALCUP_FIXTURE_CHILDREN=1 GOALCUP_FIXTURE_PIDFILE="$out" "$helper" &
# The root stays inside `wait`, so the test terminates a LIVE registered root, which is the
# only state in which a group is authority at all.
wait
