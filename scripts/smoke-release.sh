#!/bin/sh
# Smoke-test one release tarball on a machine that can run it:
#
#   sh scripts/smoke-release.sh <tarball> <version>
#
# Unpacks it and, in an environment built from nothing, checks that both binaries print
# `<name> <version>`, that `goalcup-verify dump` of PRD-01's worked example gives the committed
# lines (compared as JSON values, as crates/goalcup-format/tests/golden.rs does), which proves
# the shipped reader on this target, and that no GOALCUP_TEST_ hook reached the shipped goalcup.
set -eu

if [ $# -ne 2 ]; then
    echo "usage: sh scripts/smoke-release.sh <tarball> <version>" >&2
    exit 2
fi
tarball=$1
version=$2
data="$(cd "$(dirname "$0")/.." && pwd)/crates/goalcup-format/tests/data"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xzf "$tarball" -C "$work"
root="$work/$(basename "$tarball" .tar.gz)"
mkdir "$work/home" "$work/entry"
run() {
    env -i HOME="$work/home" PATH=/usr/bin:/bin "$@"
}

for binary in goalcup goalcup-verify; do
    printed=$(run "$root/bin/$binary" --version)
    if [ "$printed" != "$binary $version" ]; then
        echo "smoke: $binary --version printed '$printed', expected '$binary $version'" >&2
        exit 1
    fi
done

# The worked example is the raw stream; a part file on disk is zstd, so compress it first.
zstd -q -c "$data/worked-example.gcr" >"$work/entry/worked-example.gcr"
run "$root/bin/goalcup-verify" dump --text "$work/entry/worked-example.gcr" >"$work/dump.jsonl"
python3 -c '
import json, sys
got = [json.loads(l) for l in open(sys.argv[1])]
want = [json.loads(l) for l in open(sys.argv[2])]
if got != want:
    sys.exit("smoke: goalcup-verify dump of the worked example differs from the committed lines")
' "$work/dump.jsonl" "$data/worked-example.dump.jsonl"

hooks=$(grep -ac GOALCUP_TEST_ "$root/bin/goalcup" || true)
if [ "$hooks" != 0 ]; then
    echo "smoke: bin/goalcup holds GOALCUP_TEST_ $hooks times; a hooked build must not ship" >&2
    exit 1
fi
echo "smoke: $(basename "$tarball") ok"
