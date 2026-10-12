#!/bin/sh
# The test gate. `sh scripts/gate.sh fast` before every commit; `sh scripts/gate.sh full`
# before a release. The last line of output is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
#
#   fast  build, fmt, clippy, then every test except the slow `full` binaries.
#   full  fast's checks, every test including `full`, the scripts' unit tests and the link check.
#
# Tests run under cargo-nextest (one process per test, in parallel). Without it the gate falls
# back to `cargo test`, which runs every tier.
set -eu

tier=${1:-}
case "$tier" in
fast | full) ;;
*)
    echo "usage: sh scripts/gate.sh fast|full" >&2
    exit 2
    ;;
esac

cd "$(dirname "$0")/.."
start=$(date +%s)

fail() {
    echo "gate $tier: FAIL"
    exit 1
}

step() {
    echo "+ $*"
    "$@" || fail
}

# The CLI tests spawn the sibling target/debug/goalcup-daemon, so the whole workspace is built
# before any test runs. A stale daemon binary would otherwise be tested.
step cargo build --offline --workspace
step cargo fmt --all --check
step cargo clippy --offline --workspace --all-targets -- -D warnings

if cargo nextest --version >/dev/null 2>&1; then
    if [ "$tier" = fast ]; then
        # nextest rejects a filter that names a binary that does not exist, so the exclusion is
        # applied only once a `full` binary does.
        if cargo nextest list --offline --workspace --list-type binaries-only \
            -E 'binary(full)' >/dev/null 2>&1; then
            step cargo nextest run --offline --workspace -E 'not binary(full)'
        else
            echo "no 'full' test binary in the workspace; running every test"
            step cargo nextest run --offline --workspace
        fi
    else
        step cargo nextest run --offline --workspace
    fi
    # nextest does not run doctests; the compile_fail one in goalcup-protocol guards a promise.
    step cargo test --offline --workspace --doc
else
    echo "cargo-nextest not found; falling back to cargo test (runs every tier, slower)"
    step cargo test --offline --workspace
fi

if [ "$tier" = full ]; then
    step python3 -m unittest discover -s scripts
    step python3 scripts/check-links.py --root .
fi

echo "gate $tier: PASS in $(($(date +%s) - start))s"
