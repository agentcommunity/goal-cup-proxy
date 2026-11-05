#!/bin/sh
# The test gate (PRD-14 section 8), the same command in a build box and in CI.
#   fast  before every merge into integration, and CI on every push
#   full  fast's steps, every #[ignore] test but by_hand_*, the release build and its hook check;
#         phase exits, before the owner's merge to main, before a tag
#   --part lint|rust|rest   only that part of fast (lint: fmt, clippy; rust: nextest; rest: the others)
#   --partition K/N         with --part rust, run only shard K of N of the nextest tests
#   --list                  print each step's command, one per line, and run nothing
# Every step prints its time; the last line is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
# No step is skipped and nothing falls back: a skip is a hidden pass.
set -eu

usage() {
    echo "usage: sh scripts/gate.sh fast|full [--part lint|rust|rest] [--partition K/N] [--list]" >&2
    exit 2
}

tier=${1:-}
case "$tier" in
fast | full) shift ;;
*) usage ;;
esac

part=
partition=
list=
while [ $# -gt 0 ]; do
    case "$1" in
    --part)
        [ $# -ge 2 ] || usage
        part=$2
        shift 2
        ;;
    --partition)
        [ $# -ge 2 ] || usage
        partition=$2
        shift 2
        ;;
    --list)
        list=1
        shift
        ;;
    *) usage ;;
    esac
done
case "$part" in
'' | lint | rust | rest) ;;
*) usage ;;
esac
case "$partition" in
'') ;;
*[!0-9/]* | /* | */ | */*/*) usage ;;
*/*) ;;
*) usage ;;
esac
[ -z "$partition" ] || [ "$part" = rust ] || usage
[ -z "$part" ] || [ "$tier" = fast ] || usage

fail() {
    echo "gate $tier: FAIL"
    exit 1
}
trap fail INT TERM HUP

if [ -z "$list" ] && [ "$(id -u)" = 0 ] && [ "${GOALCUP_GATE_ALLOW_ROOT:-}" != 1 ]; then
    echo "gate.sh: do not run the gate as root; some permission tests always fail as root." >&2
    echo "Run it as a normal user, or set GOALCUP_GATE_ALLOW_ROOT=1 to run it anyway." >&2
    fail
fi

cd "$(dirname "$0")/.."
start=$(date +%s)

# want PART: is this part selected (all of them when --part is not given)?
want() { [ -z "$part" ] || [ "$part" = "$1" ]; }

step() {
    if [ -n "$list" ]; then
        echo "$*"
        return 0
    fi
    echo "+ $*"
    t0=$(date +%s)
    "$@" || fail
    echo "  ($(($(date +%s) - t0))s)"
}

py311= # the pad's tests need Python 3.11 or later (tomllib); the build box ships 3.9
if want rest; then
    for py in python3.11 python3; do
        if command -v "$py" >/dev/null 2>&1 && "$py" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
            py311=$py
            break
        fi
    done
fi

if [ -z "$list" ]; then
# Every missing prerequisite of the selected part, named in one line, before any step.
missing=
if want rust; then
    cargo nextest --version >/dev/null 2>&1 || missing="$missing cargo-nextest"
    command -v zstd >/dev/null 2>&1 || missing="$missing zstd"
fi
if want rest; then
    for tool in zstd python3 git npm; do
        command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
    done
    [ -d server/worker/node_modules ] || missing="$missing server/worker/node_modules(npm ci --prefix server/worker)"
fi
if want rest; then
    [ -n "$py311" ] || missing="$missing python3.11(install it, for example: sudo dnf install python3.11)"
fi
[ -z "$missing" ] || { echo "gate.sh: missing:$missing"; fail; }

# One cargo job at a time in one build box (the gate and the pad's rebuild step).
lock=target/.build-lock
mkdir -p target
while ! mkdir "$lock" 2>/dev/null; do
    [ -s "$lock/pid" ] || sleep 1 # a new holder writes its pid just after mkdir
    holder=$(cat "$lock/pid" 2>/dev/null || true)
    if [ -z "$holder" ] || ! kill -0 "$holder" 2>/dev/null; then
        echo "gate.sh: taking over the build lock of dead pid ${holder:-unknown}"
        rm -rf "$lock"
    else
        echo "gate.sh: waiting for the build lock (pid ${holder:-unknown})"
        sleep 5
    fi
done
echo $$ >"$lock/pid"
trap 'rm -rf "$lock"' EXIT
fi
py311=${py311:-python3.11} # --list on a box without it still names the command

if want lint; then
    step cargo fmt --all --check
    step cargo clippy --locked --workspace --all-targets -- -D warnings
fi
if want rust; then
    if [ -n "$partition" ]; then
        step cargo nextest run --locked --workspace --partition "hash:$partition"
    else
        step cargo nextest run --locked --workspace
    fi
fi
if want rest; then
step cargo test --locked --workspace --doc # nextest does not run doctests
step npm --prefix server/worker test
if command -v initdb >/dev/null 2>&1 || ls /usr/lib/postgresql/*/bin/initdb >/dev/null 2>&1; then step sh server/worker/scripts/check-sql.sh; else [ -n "$list" ] || echo "gate.sh: SQL scenarios skipped (no initdb)"; fi
# A named list, not `discover -s scripts`: old script tests name the old crates. A new script test
# joins only if it guards one of the seven points, the format, masking or the never-stop rules (B10).
step python3 -m unittest scripts/test_install.py scripts/test_fetch_run.py scripts/test_gate.py
step "$py311" -m unittest discover -s scripts/pad -p 'test_*.py'
step python3 scripts/check-links.py
# No personal path in tracked files: only the stand-in user names listed below are allowed.
step sh -c "! git grep -n -E '/(Users|home)/[A-Za-z0-9_.-]+' | grep -v -E '/(Users|home)/(Shared|placeholder|user|p|you|h|u|x|runs|participant|secret-root|hermes|opencode|openclaw|claude|codex)([^A-Za-z0-9_.-]|\$)'"
fi

if [ "$tier" = full ]; then
    # Fails until the first #[ignore] test (e2e_run.rs or upload_worker.rs) merges in phase 2; phase 1
    # needs only fast. No --no-tests: nextest fails when none run, so a missing e2e_run.rs cannot pass.
    step cargo nextest run --locked --workspace --run-ignored only \
        -E 'not test(/^by_hand_/)'
    step cargo build --release --locked -p goalcup -p goalcup-verify
    hooks=$(grep -ac GOALCUP_TEST_ target/release/goalcup || true)
    echo "+ GOALCUP_TEST_ in target/release/goalcup: $hooks"
    [ "$hooks" = 0 ] || fail
    # The hooked debug build must hold the text, else the clean count above proves nothing.
    hooked=$(grep -ac GOALCUP_TEST_ target/debug/goalcup || true)
    echo "+ GOALCUP_TEST_ in target/debug/goalcup: $hooked"
    [ "${hooked:-0}" -gt 0 ] || { echo "gate.sh: target/debug/goalcup has no GOALCUP_TEST_ text, so the release grep proves nothing; waits on a hook reachable from main (PRD-06 calls Policy::from_env)"; fail; }
fi

[ -z "$list" ] || exit 0
echo "gate $tier: PASS in $(($(date +%s) - start))s"
