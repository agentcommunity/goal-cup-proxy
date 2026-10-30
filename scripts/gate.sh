#!/bin/sh
# The test gate, two tiers, the same command in a build box and in CI.
#
#   sh scripts/gate.sh fast   before every merge into integration, and CI on every push
#   sh scripts/gate.sh full   phase exits, before the owner's merge to main, before a tag
#
#   fast  fmt, clippy, every test of the workspace (nextest), doctests, the Worker's tests and
#         the kept script tests.
#   full  fast's steps, then every #[ignore] test except by_hand_* (run once by hand, never in a
#         tier), the release build of the two shipped binaries without dev-dependencies, a
#         check that no GOALCUP_TEST_ hook reached the shipped goalcup, and that the hooked
#         target/debug/goalcup does hold them (else that check proves nothing).
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
# The pad's tests need Python 3.11 or later (tomllib). The build box ships 3.9, so 3.11 is installed
# there when no such Python is found.
py311=
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
# test_fetch_run.py builds nothing (no database, R2, zstd or verifier). The pad's tests are the
# rig selftest (ledger B9).
step python3 -m unittest scripts/test_install.py scripts/test_fetch_run.py
step "$py311" -m unittest discover -s scripts/pad -p 'test_*.py'

if [ "$tier" = full ]; then
    # Every other #[ignore] test (end to end, the real Worker). No --no-tests flag: nextest fails
    # when none run, so a renamed, un-ignored or missing e2e_run.rs or upload_worker.rs cannot
    # pass. Until the first of them merges (phase 2) the full tier fails here; phase 1 needs fast.
    step cargo nextest run --locked --workspace --run-ignored only \
        -E 'not test(/^by_hand_/)'
    # The binaries the pad copies, built after the test builds and without dev-dependencies.
    step cargo build --release --locked -p goalcup -p goalcup-verify
    hooks=$(grep -ac GOALCUP_TEST_ target/release/goalcup || true)
    echo "+ GOALCUP_TEST_ in target/release/goalcup: $hooks"
    [ "$hooks" = 0 ] || fail
    # The nextest steps built target/debug/goalcup with dev-dependencies, so it holds the hook
    # text once a hook is reachable from main. Without it the clean count above proves nothing
    # (PRD-05 amendment). The seam: PRD-06's uploader must call Policy::from_env.
    hooked=$(grep -ac GOALCUP_TEST_ target/debug/goalcup || true)
    echo "+ GOALCUP_TEST_ in target/debug/goalcup: $hooked"
    [ "${hooked:-0}" -gt 0 ] || { echo "gate.sh: target/debug/goalcup has no GOALCUP_TEST_ text, so the release grep proves nothing; waits on a hook reachable from main (PRD-06 calls Policy::from_env)"; fail; }
fi

echo "gate $tier: PASS in $(($(date +%s) - start))s"
