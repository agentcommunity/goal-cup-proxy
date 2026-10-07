#!/bin/sh
# The measured shape, `docs/qualification/2026-09-15-codex-recon.md` §3: a registered root
# wrapper, the native `codex` binary as an INTERIOR node (a child of the root, and the
# parent of everything else), and every child `codex` spawns in a process group of its own.
#
#   this script (root, registered)
#     codex (native, interior -- comm == "codex")
#       child 1   ** own process group **
#       child 2   ** own process group **, one of them `sleep 300`
#       child 3   ** own process group **
#
# `codex` here is `tests/support/tree.rs`'s hard-linked test-binary helper: no `setsid(1)`
# on macOS and a vacuous `set -m` under Linux's dash both rule out a pure-shell escape, so
# the helper does it in Rust (`Command::process_group(0)`) and this script only has to
# start it under a name that leaves `codex` in `ProcInfo::comm`.
#
# $1 = the hard-linked "codex" helper (its full path; `comm` keeps only the basename, see
#      `crates/goalcup-protocol/src/process.rs`'s `snapshot`).
# $2 = the pidfile the helper's `native-interior` role writes its three children's pids to.
codex_bin="$1"
children_pidfile="$2"
GOALCUP_FIXTURE_ROLE=native-interior GOALCUP_FIXTURE_PIDFILE="$children_pidfile" "$codex_bin" &
# The root stays inside `wait` for the same reason every other fixture here does: a shell
# blocked on a foreground command defers a trapped signal until that command returns, and a
# shell that ran off the end of its script would have exited before the signal ever arrived.
wait
