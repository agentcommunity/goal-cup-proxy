#!/bin/sh
# Ported from `docs/qualification/2026-09-16-review-evidence/f0-repro/fixtures/root.sh`
# (the shell variant; read-only in that directory, evidence rather than a test) — the F0
# adoption-window repro, now a real fixture this file's own tests can chase.
#
# The registered root. The member is `tests/common/support/tree.rs`'s
# `member-forks-after-root` role rather than a shell script: that function's own doc
# explains why the delay it used to run as a forked, TERM-ignoring shell helper is now an
# in-process `std::thread::sleep` instead, and this file no longer carries the shell member
# it replaced.
#
# $1 = the compiled test binary (`std::env::current_exe()`), re-exec'd under the member
#      role by setting its environment, not its argv (`tree.rs`'s ctor never touches
#      `std::env::args()`).
# $2 = the pidfile the member role forwards its forked orphan's pid to.
#
# On SIGTERM: a delayed exit (120ms), long enough that the initial SIGTERM loop in
# `terminate_within` can signal both this root and the member while the root is still
# `Anchor::Rooted`, but comfortably inside one extra `LIVENESS_POLL_MS` (50ms) cycle, and
# well before the member gets around to forking its own child (the member waits ~400ms
# before forking).
member_bin="$1"
pidfile="$2"
GOALCUP_FIXTURE_ROLE=member-forks-after-root GOALCUP_FIXTURE_PIDFILE="$pidfile" "$member_bin" &
trap 'sleep 0.12; exit 0' TERM
wait
