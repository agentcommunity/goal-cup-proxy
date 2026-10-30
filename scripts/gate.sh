#!/bin/sh
# The test gate (PRD-14 section 8), the same command in a build box and in CI.
#   fast  before every merge into integration, and CI on every push
#   full  fast's steps, every #[ignore] test but by_hand_*, the release build and its hook check;
#         phase exits, before the owner's merge to main, before a tag
# Every step prints its time; the last line is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
# No step is skipped and nothing falls back: a skip is a hidden pass.
set -eu

tier=${1:-}
case "$tier" in
fast | full) ;;
*)
    echo "usage: sh scripts/gate.sh fast|full" >&2
    exit 2
    ;;
esac

fail() {
    echo "gate $tier: FAIL"
    exit 1
}

if [ "$(id -u)" = 0 ] && [ "${GOALCUP_GATE_ALLOW_ROOT:-}" != 1 ]; then
    echo "gate.sh: do not run the gate as root; some permission tests always fail as root." >&2
    echo "Run it as a normal user, or set GOALCUP_GATE_ALLOW_ROOT=1 to run it anyway." >&2
    fail
fi

cd "$(dirname "$0")/.."
start=$(date +%s)

step() {
    echo "+ $*"
    t0=$(date +%s)
    "$@" || fail
    echo "  ($(($(date +%s) - t0))s)"
}

# Every missing prerequisite, named in one line, before any step.
missing=
cargo nextest --version >/dev/null 2>&1 || missing="$missing cargo-nextest"
for tool in zstd python3 git npm; do
    command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
done
[ -d server/worker/node_modules ] || missing="$missing server/worker/node_modules(npm ci --prefix server/worker)"
py311= # the pad's tests need Python 3.11 or later (tomllib); the build box ships 3.9
for py in python3.11 python3; do
    if command -v "$py" >/dev/null 2>&1 && "$py" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
        py311=$py
        break
    fi
done
if [ -z "$py311" ] && command -v dnf >/dev/null 2>&1 && sudo -n dnf install -y -q python3.11 >/dev/null 2>&1; then
    py311=python3.11
fi
[ -n "$py311" ] || missing="$missing python3.11"
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
trap fail INT TERM HUP

step cargo fmt --all --check
step cargo clippy --locked --workspace --all-targets -- -D warnings
step cargo nextest run --locked --workspace
step cargo test --locked --workspace --doc # nextest does not run doctests
step npm --prefix server/worker test
# A named list, not `discover -s scripts`: old script tests name the old crates. A new script test
# joins only if it guards one of the seven points, the format, masking or the never-stop rules (B10).
step python3 -m unittest scripts/test_install.py scripts/test_fetch_run.py
step "$py311" -m unittest discover -s scripts/pad -p 'test_*.py'

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

echo "gate $tier: PASS in $(($(date +%s) - start))s"
