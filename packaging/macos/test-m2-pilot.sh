#!/bin/sh
# Focused installed-path smoke for the private M2 local-capture candidate. It uses a
# disposable tracker prefix and a tiny synthetic Codex-compatible executable; it never
# starts a real harness or sends traffic to a model service.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd -P)

fail() {
    echo "test-m2-pilot.sh: FAIL: $1" >&2
    exit 1
}

step() {
    echo "test-m2-pilot.sh: -- $1"
}

[ "$(uname -s)" = "Darwin" ] || fail "this check only runs on macOS"
case "$(uname -m)" in
    arm64) target="aarch64-apple-darwin"; arch_label="macos/aarch64" ;;
    x86_64) target="x86_64-apple-darwin"; arch_label="macos/x86_64" ;;
    *) fail "unsupported host architecture: $(uname -m)" ;;
esac

# Resolve macOS's temporary-directory symlink before staging; the packager refuses every
# symlink in its output-parent chain by design.
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/goalcup-test-m2-pilot.XXXXXX")
work_dir=$(CDPATH= cd -- "$work_dir" && pwd -P)
prefix="$work_dir/prefix"
stage="$work_dir/stage"
label="com.agentcommunity.goalcup.m2pilot.$$"
prepublish_prefix="$work_dir/prepublish-prefix"
prepublish_label="com.agentcommunity.goalcup.m2pilot.pre.$$"
postpublish_prefix="$work_dir/postpublish-prefix"
postpublish_label="com.agentcommunity.goalcup.m2pilot.post.$$"
home="$work_dir/home"
fake_bin="$work_dir/harness-bin"
host_path=$PATH
mkdir -m 0700 "$home" "$fake_bin"
installed=0
fixture_daemon_pid=""
cleanup_failed=0

stop_fixture_daemon() {
    [ -n "$fixture_daemon_pid" ] || return 0
    pid=$fixture_daemon_pid
    kill -TERM "$pid" 2>/dev/null || true
    stop_attempt=0
    while kill -0 "$pid" 2>/dev/null; do
        stop_attempt=$((stop_attempt + 1))
        if [ "$stop_attempt" -ge 40 ]; then
            kill -KILL "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
            fixture_daemon_pid=""
            return 1
        fi
        sleep 0.25
    done
    if wait "$pid" 2>/dev/null; then
        stop_status=0
    else
        stop_status=$?
    fi
    fixture_daemon_pid=""
    [ "$stop_status" -eq 0 ]
}

bootout_disposable_label() {
    owned_label=$1
    case "$owned_label" in
        com.agentcommunity.goalcup.m2pilot.*) ;;
        *)
            echo "test-m2-pilot.sh: cleanup refused non-disposable label: $owned_label" >&2
            return 1
            ;;
    esac
    if launchctl print "gui/$(id -u)/$owned_label" >/dev/null 2>&1; then
        launchctl bootout "gui/$(id -u)/$owned_label" >/dev/null 2>&1 || {
            echo "test-m2-pilot.sh: cleanup could not unload $owned_label" >&2
            return 1
        }
    fi
}

cleanup() {
    status=$?
    stop_fixture_daemon >/dev/null 2>&1 || cleanup_failed=1
    if [ "$installed" -eq 1 ]; then
        HOME="$home" sh "$SCRIPT_DIR/uninstall-assisted.sh" --prefix "$prefix" --label "$label" >/dev/null 2>&1 || cleanup_failed=1
    fi
    if [ -f "$postpublish_prefix/.goalcup-install-manifest.json" ]; then
        HOME="$home" sh "$SCRIPT_DIR/uninstall-assisted.sh" --prefix "$postpublish_prefix" \
            --label "$postpublish_label" >/dev/null 2>&1 || cleanup_failed=1
    fi
    # Cover every label minted by this pilot. The helper refuses anything outside the
    # disposable prefix before inspecting or unloading the exact gui/<uid>/<label> job.
    bootout_disposable_label "$label" || cleanup_failed=1
    bootout_disposable_label "$prepublish_label" || cleanup_failed=1
    bootout_disposable_label "$postpublish_label" || cleanup_failed=1
    rm -f "$home/Library/LaunchAgents/$label.plist"
    rm -f "$home/Library/LaunchAgents/$prepublish_label.plist"
    rm -f "$home/Library/LaunchAgents/$postpublish_label.plist"
    rm -rf "$work_dir"
    if [ "$status" -eq 0 ] && [ "$cleanup_failed" -ne 0 ]; then
        status=1
    fi
    exit "$status"
}
trap cleanup EXIT INT TERM

step "building the three shipping binaries from this source revision"
target_dir="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
safe_host_path=$(python3 - "$host_path" "$REPO_ROOT" "$target_dir" <<'PYEOF'
import os, sys
raw_path, repo_root, target_dir = sys.argv[1:]
repo_root = os.path.realpath(repo_root)
if not os.path.isabs(target_dir):
    target_dir = os.path.join(repo_root, target_dir)
target_dir = os.path.realpath(target_dir)

def within(path, root):
    try:
        return os.path.commonpath((path, root)) == root
    except ValueError:
        return False

kept = []
for item in raw_path.split(os.pathsep):
    if not item:
        continue
    resolved = os.path.realpath(item)
    if within(resolved, repo_root) or within(resolved, target_dir):
        continue
    kept.append(item)
print(os.pathsep.join(kept) or "/usr/bin:/bin:/usr/sbin:/sbin")
PYEOF
)
(
    cd "$REPO_ROOT"
    cargo build --release --offline -p goalcup-cli -p goalcup-daemon -p goalcup-verifier
) || fail "cargo build failed"
binaries_dir=$(CDPATH= cd -- "$target_dir/release" && pwd -P)
for name in goalcup goalcup-daemon goalcup-verify; do
    [ -x "$binaries_dir/$name" ] || fail "built binary missing or not executable: $name"
done
revision=$(cd "$REPO_ROOT" && git rev-parse HEAD)

step "staging the explicitly private, unsigned, unnotarized, unqualified m2-pilot candidate"
python3 "$REPO_ROOT/scripts/package-release.py" --stage --profile m2-pilot \
    --repo "$REPO_ROOT" --binaries "$binaries_dir" --output "$stage" \
    --target "$target" --revision "$revision" --version "0.0.0-m2-pilot-candidate" \
    || fail "m2-pilot staging failed"

python3 - "$stage/bundle.json" "$arch_label" <<'PYEOF'
import json, sys
manifest = json.load(open(sys.argv[1], encoding="utf-8"))
assert manifest["profile"] == "m2-pilot"
assert manifest["target"] == {"macos/aarch64": "aarch64-apple-darwin", "macos/x86_64": "x86_64-apple-darwin"}[sys.argv[2]]
assert manifest["candidate"] is True and manifest["distribution"] == "private"
assert manifest["qualification"] == "unqualified" and manifest["signature"] == "unsigned"
assert manifest["labels"] == sorted({"m2-pilot", "candidate", "private", "unsigned", "unnotarized", "unqualified", sys.argv[2]})
assert set(manifest["profile_sha256"]) == {"codex-cli", "claude-code-cli"}
inventory = {item["path"]: item["sha256"] for item in manifest["files"]}
for connector, digest in manifest["profile_sha256"].items():
    assert digest == inventory[f"connectors/{connector}/harness.toml"]
for item in manifest["files"]:
    print(f"test-m2-pilot.sh: sha256 {item['sha256']}  {item['path']}")
print(f"test-m2-pilot.sh: source {manifest['revision']} target {manifest['target']} version {manifest['version']}")
PYEOF

step "proving pre-publication failure leaves no visible install or launchd job"
set +e
HOME="$home" GOALCUP_INSTALL_TESTING=1 GOALCUP_INSTALL_TEST_FAIL=copy \
    sh "$SCRIPT_DIR/install-assisted.sh" --profile m2-pilot \
    --bundle "$stage" --prefix "$prepublish_prefix" --label "$prepublish_label" \
    >"$work_dir/prepublish.out" 2>"$work_dir/prepublish.err"
prepublish_status=$?
set -e
[ "$prepublish_status" -ne 0 ] || fail "injected pre-publication failure unexpectedly succeeded"
[ ! -e "$prepublish_prefix" ] || fail "injected pre-publication failure exposed a prefix"
[ ! -e "$home/Library/LaunchAgents/$prepublish_label.plist" ] \
    || fail "injected pre-publication failure exposed a plist"
[ ! -s "$work_dir/prepublish.out" ] || fail "injected pre-publication failure printed a success banner"
if launchctl print "gui/$(id -u)/$prepublish_label" >/dev/null 2>&1; then
    fail "injected pre-publication failure left a launchd job"
fi

step "proving post-publication failure preserves outputs and a ready job for manual recovery"
set +e
HOME="$home" GOALCUP_INSTALL_TESTING=1 GOALCUP_INSTALL_TEST_FAIL=after-activation \
    sh "$SCRIPT_DIR/install-assisted.sh" --profile m2-pilot \
    --bundle "$stage" --prefix "$postpublish_prefix" --label "$postpublish_label" \
    >"$work_dir/postpublish.out" 2>"$work_dir/postpublish.err"
postpublish_status=$?
set -e
[ "$postpublish_status" -ne 0 ] || fail "injected post-publication failure unexpectedly succeeded"
[ -d "$postpublish_prefix" ] || fail "post-publication failure removed the published prefix"
[ -f "$home/Library/LaunchAgents/$postpublish_label.plist" ] \
    || fail "post-publication failure removed the published plist"
launchctl print "gui/$(id -u)/$postpublish_label" >/dev/null 2>&1 \
    || fail "post-publication failure removed or failed to preserve the ready launchd job"
[ ! -s "$work_dir/postpublish.out" ] || fail "post-publication failure printed a success banner"
grep -F "$postpublish_prefix" "$work_dir/postpublish.err" >/dev/null \
    || fail "post-publication recovery report omitted the exact prefix"
grep -F "$home/Library/LaunchAgents/$postpublish_label.plist" "$work_dir/postpublish.err" >/dev/null \
    || fail "post-publication recovery report omitted the exact plist"
grep -F "launchd label: $postpublish_label" "$work_dir/postpublish.err" >/dev/null \
    || fail "post-publication recovery report omitted the exact label"
grep -F "uid domain: gui/$(id -u)" "$work_dir/postpublish.err" >/dev/null \
    || fail "post-publication recovery report omitted the uid domain"
grep -F "bootstrap returned success: yes" "$work_dir/postpublish.err" >/dev/null \
    || fail "post-publication recovery report omitted bootstrap state"
grep -F "readiness observed: yes" "$work_dir/postpublish.err" >/dev/null \
    || fail "post-publication recovery report omitted readiness state"
HOME="$home" sh "$SCRIPT_DIR/uninstall-assisted.sh" --prefix "$postpublish_prefix" \
    --label "$postpublish_label" >/dev/null \
    || fail "manual recovery cleanup of injected post-publication install failed"

step "installing only the m2-pilot profile under the disposable prefix"
HOME="$home" sh "$SCRIPT_DIR/install-assisted.sh" --profile m2-pilot \
    --bundle "$stage" --prefix "$prefix" --label "$label" \
    || fail "m2-pilot installation failed"
installed=1
config_file="$prefix/etc/daemon.toml"
[ -f "$config_file" ] || fail "installer did not write daemon config"
if python3 - "$config_file" <<'PYEOF'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
raise SystemExit(not bool(re.search(r"^(upload_url|server_public_key)\s*=", text, re.M)))
PYEOF
then
    fail "local M2 config contains a service URL or signing key"
fi

cat > "$fake_bin/codex" <<'SHIM'
#!/bin/sh
if [ "${1:-}" = "--version" ]; then
    echo "codex-cli 0.153.0"
    exit 0
fi
printf '{"type":"thread.started","thread_id":"m2-pilot-smoke"}\n'
printf 'm2-capture-ok' > result.txt
test "$(cat result.txt)" = "m2-capture-ok"
printf '{"type":"turn.completed"}\n'
SHIM
cat > "$fake_bin/claude" <<'SHIM'
#!/bin/sh
if [ "${1:-}" = "--version" ]; then
    echo "2.1.283"
    exit 0
fi
exit 2
SHIM
chmod 0700 "$fake_bin/codex" "$fake_bin/claude"
export HOME="$home"
export GOALCUP_CONFIG_DIR="$prefix/etc"
export GOALCUP_CONNECTORS_DIR="$prefix/share/goalcup/connectors"
export PATH="$prefix/bin:$safe_host_path"

step "checking installed binaries, profile digests, and both connector doctor paths"
for name in goalcup goalcup-daemon goalcup-verify; do
    [ -x "$prefix/bin/$name" ] || fail "installed binary missing: $name"
    case "$(command -v "$name" 2>/dev/null || true)" in
        "$prefix/bin/$name") ;;
        *) fail "PATH did not resolve $name from the disposable install" ;;
    esac
done
python3 - "$stage/bundle.json" "$prefix" <<'PYEOF'
import hashlib, json, pathlib, sys
manifest = json.load(open(sys.argv[1], encoding="utf-8"))
prefix = pathlib.Path(sys.argv[2])
for item in manifest["files"]:
    parts = item["path"].split("/")
    if parts[0] == "bin":
        installed = prefix / "bin" / parts[1]
    elif parts[0] == "connectors":
        installed = prefix / "share" / "goalcup" / "connectors"
        for part in parts[1:]:
            installed /= part
    else:
        raise AssertionError(f"unexpected packaged path {item['path']}")
    digest = hashlib.sha256(installed.read_bytes()).hexdigest()
    assert digest == item["sha256"], (item["path"], digest, item["sha256"])
    assert installed.stat().st_size == item["size"], item["path"]
PYEOF
"$prefix/bin/goalcup" --version >/dev/null
"$prefix/bin/goalcup" --help >/dev/null
"$prefix/bin/goalcup-daemon" --version >/dev/null
"$prefix/bin/goalcup-daemon" --help >/dev/null
"$prefix/bin/goalcup-verify" --version >/dev/null
"$prefix/bin/goalcup-verify" --help >/dev/null
attempt=0
while ! "$prefix/bin/goalcup-daemon" --config "$config_file" --print-addresses >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 40 ] || fail "installed daemon did not become ready"
    sleep 0.25
done
for connector in codex-cli claude-code-cli; do
    "$prefix/bin/goalcup" doctor --json --harness "$connector" \
        >"$work_dir/doctor-$connector.json" \
        || fail "installed doctor failed for $connector"
done

# Preserve the shipped profiles and their real executable pins for package/version proof.
# The synthetic lifecycle below gets a separately named, explicitly unpinned test profile;
# production pin checks stay enabled and the fake executable can never satisfy codex-cli.
step "preparing an isolated controlled-workload profile with an exact shim pin"
fixture_connectors="$work_dir/controlled-connectors"
fixture_codex="$fixture_connectors/controlled-workload-codex"
mkdir -m 0700 -p "$fixture_codex"
cp -p "$prefix/share/goalcup/connectors/codex-cli/harness.toml" "$fixture_codex/harness.toml"
python3 - "$fixture_codex/harness.toml" "$fake_bin/codex" <<'PYEOF'
import hashlib, pathlib, re, sys
harness = pathlib.Path(sys.argv[1])
text = harness.read_text(encoding="utf-8")
digest = hashlib.sha256(pathlib.Path(sys.argv[2]).read_bytes()).hexdigest()
text, count = re.subn(r'(?m)^id\s*=\s*"codex-cli"\s*$', 'id = "controlled-workload-codex"', text, count=1)
assert count == 1, "expected exactly one shipped Codex connector id"
text, count = re.subn(r'(?m)^strategy[ \t]*=[ \t]*"native_pid"(?:[ \t]+#.*)?[ \t]*$', 'strategy = "process_group"', text, count=1)
assert count == 1, "expected the shipped Codex process strategy"
text, count = re.subn(r'(?m)^native_binary\s*=\s*"codex"\s*\n', "", text, count=1)
assert count == 1, "expected the shipped Codex native process identity"
text, count = re.subn(r'(?m)^(sha256\s*=\s*)"[0-9a-f]{64}"$', rf'\g<1>"{digest}"', text, count=1)
assert count == 1, "expected the measured Codex capture executable pin"
harness.write_text(text, encoding="utf-8")
print(f"test-m2-pilot.sh: synthetic schema profile controlled-workload-codex (process_group) pins codex shim sha256 {digest}; shipped profiles remain unchanged")
PYEOF

# Keep the fixture runtime state separate from the launchd-served install config and roots.
# The package/installer path above has already loaded and served the installed daemon once;
# after its scoped unload, run that same installed binary directly in foreground for the
# controlled profile instead of rewriting/rebootstrapping the same launchd label.
fixture_config="$work_dir/controlled-daemon.toml"
fixture_config_dir="$work_dir/controlled-config"
fixture_artifact_root="$prefix/var/controlled-artifacts"
fixture_workspace_parent="$prefix/var/controlled-workspaces"
mkdir -m 0700 -p "$fixture_config_dir" "$fixture_artifact_root" "$fixture_workspace_parent"
cat > "$fixture_config" <<EOF
artifact_root = "$fixture_artifact_root"
connectors_dir = "$fixture_connectors"
config_dir = "$fixture_config_dir"
workspace_parent = "$fixture_workspace_parent"
control_addr = "127.0.0.1:0"
EOF
chmod 0600 "$fixture_config"
launchctl bootout "gui/$(id -u)/$label" >/dev/null 2>&1 \
    || fail "could not stop the disposable launchd daemon before fixture foreground start"

export GOALCUP_CONFIG_DIR="$fixture_config_dir"
export GOALCUP_CONNECTORS_DIR="$fixture_connectors"
export PATH="$fake_bin:$prefix/bin:/usr/bin:/bin:/usr/sbin:/sbin"

step "starting the installed daemon in foreground with isolated fixture roots"
"$prefix/bin/goalcup-daemon" --config "$fixture_config" --foreground \
    >"$work_dir/controlled-daemon.log" 2>&1 &
fixture_daemon_pid=$!
attempt=0
while ! "$prefix/bin/goalcup-daemon" --config "$fixture_config" --print-addresses >/dev/null 2>&1; do
    if ! kill -0 "$fixture_daemon_pid" 2>/dev/null; then
        if wait "$fixture_daemon_pid" 2>/dev/null; then :; else :; fi
        fixture_daemon_pid=""
        fail "foreground installed daemon exited before readiness: $(cat "$work_dir/controlled-daemon.log")"
    fi
    attempt=$((attempt + 1))
    [ "$attempt" -lt 40 ] || fail "foreground installed daemon did not publish its fresh control file"
    sleep 0.25
done

step "driving an explicitly unpinned synthetic --dev v5 lifecycle and its deliberate coverage gap"
mkdir -m 0700 "$work_dir/run"
printf '%s' 'Create result.txt with exactly m2-capture-ok and read it back.' > "$work_dir/run/goal.txt"
start_output="$work_dir/start.out"
"$prefix/bin/goalcup" start --dev --harness controlled-workload-codex \
    --goal-file "$work_dir/run/goal.txt" --capture-reserve-bytes 67108864 \
    >"$start_output" 2>&1 || fail "installed v5 start failed: $(cat "$start_output")"
cat "$start_output"
entry=$(sed -n 's/^entry //p' "$start_output" | head -n 1)
[ -n "$entry" ] || fail "start output did not name the entry"
trace="$fixture_artifact_root/artifacts/$entry/trace"
[ -d "$trace" ] || fail "installed capture trace missing from disposable artifact-root layout: $trace"

"$prefix/bin/goalcup" status "$entry" --json > "$work_dir/status.json" \
    || fail "installed status command failed"
python3 - "$work_dir/status.json" <<'PYEOF'
import json, sys
status = json.load(open(sys.argv[1], encoding="utf-8"))
payload = status.get("payload", status)
capture = payload.get("capture")
assert capture is not None, "status omitted the M2 capture fields"
assert capture["run"] == "sealed", capture
assert capture["delivery"] == "local_only", capture
assert capture["anchor"] == "local_only", capture
PYEOF

set +e
"$prefix/bin/goalcup-verify" --capture-integrity "$trace" \
    >"$work_dir/integrity.json" 2>"$work_dir/integrity.err"
integrity_exit=$?
set -e
[ "$integrity_exit" -eq 1 ] || fail "deliberate no-wire/no-native gap expected integrity exit 1, got $integrity_exit"
python3 - "$work_dir/integrity.json" <<'PYEOF'
import json, sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["integrity"] == "incomplete", report
assert report["sealed"] is True, report
assert report["coverage_incomplete"] is True, report
assert report["anchor"] == "local_only", report
PYEOF

step "uninstalling and confirming the disposable prefix alone was used"
stop_fixture_daemon || fail "foreground installed daemon did not stop cleanly"
HOME="$home" sh "$SCRIPT_DIR/uninstall-assisted.sh" --prefix "$prefix" --label "$label" \
    || fail "m2-pilot uninstall failed"
installed=0
[ ! -e "$prefix/bin" ] || fail "uninstall left installed binaries behind"
[ -d "$trace" ] || fail "uninstall removed the disposable trace"
echo "test-m2-pilot.sh: PASS (candidate local smoke; deliberate coverage gap, integrity exit 1)"
