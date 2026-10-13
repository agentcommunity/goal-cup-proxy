#!/bin/sh
# Smoke-test one release tarball on a machine that can run it:
#
#   sh scripts/smoke-release.sh <tarball> <version>
#
# Unpacks it, checks that all three binaries print <version>, and checks through
# `goalcup doctor --json` what the Homebrew formula's test checks: the unpacked goalcup finds
# the connectors it ships, and every one's pin passes. doctor exits 1 when no pinned harness is
# installed, so only its report is judged. It runs with an environment built from nothing, so
# neither $GOALCUP_CONNECTORS_DIR nor a real config directory can stand in for the tarball's.
set -eu

if [ $# -ne 2 ]; then
    echo "usage: sh scripts/smoke-release.sh <tarball> <version>" >&2
    exit 2
fi
tarball=$1
version=$2

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xzf "$tarball" -C "$work"
root="$work/$(basename "$tarball" .tar.gz)"
mkdir "$work/home"
run() {
    env -i HOME="$work/home" PATH=/usr/bin:/bin GOALCUP_CONFIG_DIR="$work/config" "$@"
}

for binary in goalcup goalcup-daemon goalcup-verify; do
    printed=$(run "$root/bin/$binary" --version)
    if [ "$printed" != "$binary $version" ]; then
        echo "smoke: $binary --version printed '$printed', expected '$binary $version'" >&2
        exit 1
    fi
done

run "$root/bin/goalcup" doctor --json >"$work/doctor.json" || true
python3 - "$work/doctor.json" "$root/share/goalcup/connectors" <<'PY'
import json, os, sys
try:
    report = json.load(open(sys.argv[1]))
except ValueError:
    sys.exit("smoke: goalcup doctor --json printed no report:\n" + open(sys.argv[1]).read())
shipped = sorted(os.listdir(sys.argv[2]))
pins = {c["connector_id"]: next((k["level"] for k in c["checks"] if k["id"] == "manifest_pin"), None)
        for c in report["connectors"]}
if sorted(pins) != shipped or set(pins.values()) != {"pass"}:
    sys.exit(f"smoke: shipped connectors {shipped}, doctor manifest_pin {pins}")
PY
echo "smoke: $(basename "$tarball") ok"
