#!/bin/sh
# The test gate, two tiers, the same command in a build box and in CI.
#
#   sh scripts/gate.sh fast   before every merge into integration, and CI on every push
#   sh scripts/gate.sh full   phase exits, before the owner's merge to main, before a tag
#
#   fast  fmt, clippy, every test of the workspace (nextest), doctests, the Worker's tests and
#         the kept script tests.
#   full  fast's steps, then every #[ignore] test except by_hand_* (run once by hand, never in a
#         tier), the release build of the two shipped binaries without dev-dependencies, and a
#         check that no GOALCUP_TEST_ hook reached the shipped goalcup.
#
# The workspace holds only the new crates; the old ones are in its `exclude` list and never
# built here. No step is skipped and nothing falls back: a skip is a hidden pass. Every step
# prints its time. The last line is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
set -eu

tier=${1:-}
case "$tier" in
fast | full) ;;
*)
    echo "usage: sh scripts/gate.sh fast|full" >&2
    exit 2
    ;;
esac

# Some permission tests always fail as root (a root process can open a file mode 000 forbids), so
# say so at once rather than after the long steps. GOALCUP_GATE_ALLOW_ROOT=1 runs it anyway.
if [ "$(id -u)" = 0 ] && [ "${GOALCUP_GATE_ALLOW_ROOT:-}" != 1 ]; then
    echo "gate.sh: do not run the gate as root; some permission tests always fail as root." >&2
    echo "Run it as a normal user, or set GOALCUP_GATE_ALLOW_ROOT=1 to run it anyway." >&2
    echo "gate $tier: FAIL"
    exit 1
fi

cd "$(dirname "$0")/.."
start=$(date +%s)

fail() {
    echo "gate $tier: FAIL"
    exit 1
}

step() {
    echo "+ $*"
    t0=$(date +%s)
    "$@" || fail
    echo "  ($(($(date +%s) - t0))s)"
}

# Every prerequisite, named in one line, before any step.
missing=
cargo nextest --version >/dev/null 2>&1 || missing="$missing cargo-nextest"
for tool in zstd python3 git npm; do
    command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
done
[ -d server/worker/node_modules ] || missing="$missing server/worker/node_modules(npm ci --prefix server/worker)"
if [ -n "$missing" ]; then
    echo "gate.sh: missing:$missing"
    fail
fi

# One cargo job at a time inside one build box (the gate and the pad's rebuild step share it).
# A dead holder's lock is taken over.
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
trap 'exit 1' INT TERM HUP

step cargo fmt --all --check
step cargo clippy --locked --workspace --all-targets -- -D warnings
step cargo nextest run --locked --workspace
# nextest does not run doctests.
step cargo test --locked --workspace --doc
step npm --prefix server/worker test
# A named list, not `discover -s scripts`: old script tests name the old crates. A new script test
# joins only if it guards one of the seven points, the format, masking or the never-stop rules.
# test_fetch_run.py joins when PRD-07's rewrite merges (today it builds the old goalcup-verifier),
# and `discover -s scripts/pad` after PRD-12's first test_pad.py merge.
step python3 -m unittest scripts/test_install.py

if [ "$tier" = full ]; then
    # Every other #[ignore] test (end to end, the real Worker). Early in the build there may be
    # none, which nextest reports as a warning.
    step cargo nextest run --locked --workspace --run-ignored only --no-tests=warn \
        -E 'not test(/^by_hand_/)'
    # The binaries the pad copies, built after the test builds and without dev-dependencies.
    step cargo build --release --locked -p goalcup -p goalcup-verify
    hooks=$(grep -ac GOALCUP_TEST_ target/release/goalcup || true)
    echo "+ GOALCUP_TEST_ in target/release/goalcup: $hooks"
    [ "$hooks" = 0 ] || fail
fi

echo "gate $tier: PASS in $(($(date +%s) - start))s"
