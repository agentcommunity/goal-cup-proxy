#!/bin/sh
# The test gate. `sh scripts/gate.sh fast` before every commit; `sh scripts/gate.sh full`
# before a release. The last line of output is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
#
#   fast  build, fmt, clippy, then every test except the slow `full` and `m2_cli_lifecycle`
#         binaries.
#   full  fast's checks, every test including `full`, the CLI's `#[ignore]`d end-to-end tests
#         (the `m2_synthetic_vertical` and `capture_failure_campaign` modules of its `full`
#         binary, about 4.5 minutes), the scripts' unit tests and the link check. The other
#         ignored tests need a real Hermes or a disk image and are run by hand.
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
        # The slow binaries the fast tier skips: every crate's `full`, and the CLI's standalone
        # end-to-end lifecycle (about 100 s). nextest rejects a filter that names a binary that
        # does not exist, so the exclusion is probed first; each operator must match a binary.
        slow='binary(full) | binary(m2_cli_lifecycle)'
        if cargo nextest list --offline --workspace --list-type binaries-only \
            -E "$slow" >/dev/null 2>&1; then
            step cargo nextest run --offline --workspace -E "not ($slow)"
        else
            echo "gate fast: WARNING: no test binary named 'full' or 'm2_cli_lifecycle' was" \
                "found (renamed?), so the fast tier is running EVERY test, slow ones included" >&2
            step cargo nextest run --offline --workspace
        fi
    else
        step cargo nextest run --offline --workspace
        # The end-to-end guarantee tests are `#[ignore]`d so a plain run skips them; the
        # release check runs them by name. The other ignored tests are not selected.
        step cargo nextest run --offline -p goalcup-cli --test full --run-ignored ignored-only \
            -E 'test(/^(m2_synthetic_vertical|capture_failure_campaign)::/)'
    fi
    # nextest does not run doctests; the compile_fail one in goalcup-protocol guards a promise.
    step cargo test --offline --workspace --doc
else
    echo "cargo-nextest not found; falling back to cargo test (runs every tier, slower)"
    step cargo test --offline --workspace
    if [ "$tier" = full ]; then
        step cargo test --offline -p goalcup-cli --test full -- --ignored \
            m2_synthetic_vertical:: capture_failure_campaign::
    fi
fi

if [ "$tier" = full ]; then
    step python3 -m unittest discover -s scripts
    step python3 scripts/check-links.py --root .
fi

echo "gate $tier: PASS in $(($(date +%s) - start))s"
