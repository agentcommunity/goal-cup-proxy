#!/bin/sh
# Smoke-test one release tarball on a machine that can run it:
#
#   sh scripts/smoke-release.sh <tarball> <version>
#
# Unpacks it, checks that all three binaries print <version>, and checks through
# `goalcup doctor --json` what the Homebrew formula's test checks: the unpacked goalcup finds
# the connectors it ships, and every one's pin passes. doctor exits 1 when no pinned harness is
# installed, so only its report is judged. Then it checks that every shipped connector can pin
# the harness on this machine's platform: the harness is not installed here, so this reads the
# manifests. A connector with `[capture.platform.*]` tables pins one native build per platform,
# its base pin being the macOS arm64 build (a convention, not a manifest field), so it must name
# this platform unless that is macos-aarch64. A connector without any is exempt: Hermes's pin is
# a console script whose shebang names one install's interpreter, so it is not portable to any
# other machine either way (docs/harnesses/hermes-cli.md, section 2). The gate holds the same
# rule in Rust (`every_per_platform_connector_pins_every_release_platform`); this is the
# release-time backstop. It runs with an environment built from nothing, so
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
case "$(uname -s) $(uname -m)" in
"Darwin arm64") platform=macos-aarch64 ;;
"Darwin x86_64") platform=macos-x86_64 ;;
"Linux x86_64") platform=linux-x86_64 ;;
"Linux aarch64" | "Linux arm64") platform=linux-aarch64 ;;
*)
    echo "smoke: no connector platform for $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac
python3 - "$root/share/goalcup/connectors" "$platform" <<'PY'
import os, re, sys
root, platform = sys.argv[1], sys.argv[2]
missing = []
for cid in sorted(os.listdir(root)):
    text = open(os.path.join(root, cid, "harness.toml")).read()
    declared = set(re.findall(r"^\[capture\.platform\.([a-z0-9_-]+)\]$", text, re.M))
    if declared and platform != "macos-aarch64" and platform not in declared:
        missing.append(cid)
if missing:
    sys.exit(f"smoke: no {platform} executable pin in: {', '.join(missing)}")
PY
echo "smoke: $(basename "$tarball") ok"
