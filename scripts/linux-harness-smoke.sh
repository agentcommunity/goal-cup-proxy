#!/bin/sh
# No-login smoke of the pinned Linux harness builds (x86_64 or arm64, from `uname -m`).
# For each harness: download its npm platform package, check the package's and the binary's
# SHA-256, put the binary first on PATH, then `goalcup doctor` must pass and one `goalcup start`
# with a never-logged-in home must launch it, stop at auth, seal and verify (exit 0), and leave
# no process named codex or claude running (so run it where no other one runs, as
# linux-check.sh's container does). Nothing here signs in; no model reply is expected.
# Needs: Linux, goalcup, goalcup-daemon and goalcup-verify on PATH, GOALCUP_CONNECTORS_DIR set
# when the binaries are not beside a connectors/ directory, TMPDIR on ext4 or XFS, curl,
# sha256sum, pgrep, and network (the npm registry, and `goalcup start` checks the provider host
# answers). `sh scripts/linux-check.sh` runs it in Docker when LINUX_CHECK_HARNESSES=1.
# Usage: sh scripts/linux-harness-smoke.sh [codex-cli] [claude-code-cli]   (default: both)
# Last line: `harness smoke: PASS` or `harness smoke: FAIL <harness> <step>`.
set -u
fail() { echo "harness smoke: FAIL $1"; exit 1; }

case $(uname -m) in
x86_64) arch=x64 ;;
aarch64) arch=arm64 ;;
*) fail "arch $(uname -m)" ;;
esac
work=$(mktemp -d) || fail "mktemp"
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT HUP TERM

# The npm tarball and the binary inside it, with both SHA-256s. The binary hashes are the
# connectors' [capture.platform.linux-*] pins; `goalcup doctor` checks those again.
package() {
    case $1-$arch in
    codex-cli-x64)
        url=https://registry.npmjs.org/@openai/codex/-/codex-0.153.0-linux-x64.tgz
        tgz=856f408ea61b44a381b7d6fb7c82365dfcef649ae2a340fc01282cf63c30cd8a
        file=package/vendor/x86_64-unknown-linux-musl/bin/codex
        bin=fce635028842bfe9257140e8b7d53162732945e2f356fc35225be0702b4974be prog=codex ;;
    codex-cli-arm64)
        url=https://registry.npmjs.org/@openai/codex/-/codex-0.153.0-linux-arm64.tgz
        tgz=1c65fda30a919908688147b67b1b195ab635bd4a5eac49ffa7be527a13af0a09
        file=package/vendor/aarch64-unknown-linux-musl/bin/codex
        bin=c92824b2e068b8cb15eecba45271f7e3dd35a5b3929383593284a36074d40aec prog=codex ;;
    claude-code-cli-x64)
        url=https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-2.1.283.tgz
        tgz=d14ec0fca400151092c926928fea7e8e38231f7a90e6e1f8eeb2a55c24676afb
        file=package/claude
        bin=1859583ce32920595c61ef868bee52e1b1594f7486db209935e01f1e5e804ae2 prog=claude ;;
    claude-code-cli-arm64)
        url=https://registry.npmjs.org/@anthropic-ai/claude-code-linux-arm64/-/claude-code-linux-arm64-2.1.283.tgz
        tgz=e5ea2a2b09ad70e447d985bdcd1fb49ac60043cd4841ddc434cd69c5c8770bf7
        file=package/claude
        bin=346d294f0103d6fc0de11ac953579b5c62dfa90698a4cfc486b6f927c615e697 prog=claude ;;
    *) fail "$1 unknown harness" ;;
    esac
}

smoke() {
    h=$1
    package "$h"
    d=$work/$h
    mkdir -p "$d/pkg" "$d/bin" "$d/home" "$d/goal" || fail "$h mkdir"
    echo "harness smoke: $h ($arch) fetch"
    curl -fsSL -o "$d/pkg.tgz" "$url" || fail "$h download"
    echo "$tgz  $d/pkg.tgz" | sha256sum -c --quiet || fail "$h tarball sha256"
    tar xzf "$d/pkg.tgz" -C "$d/pkg" || fail "$h extract"
    echo "$bin  $d/pkg/$file" | sha256sum -c --quiet || fail "$h binary sha256"
    ln -s "$d/pkg/$file" "$d/bin/$prog" || fail "$h link"

    # A home and a tracker directory nobody signed in to, both under TMPDIR (the trace folder
    # is <GOALCUP_CONFIG_DIR>-artifacts beside it). Every harness call, `--version` included,
    # runs here: Codex writes into its home even for `--version`. Unset what could name a login.
    run() {
        env -u CODEX_HOME -u CLAUDE_CONFIG_DIR -u OPENAI_API_KEY -u ANTHROPIC_API_KEY \
            HOME="$d/home" GOALCUP_CONFIG_DIR="$d/cfg" PATH="$d/bin:$PATH" "$@"
    }
    echo "harness smoke: $h version: $(run "$prog" --version 2>&1 | head -n 1)"
    echo "Print the word hello." >"$d/goal/goal.txt"
    run goalcup doctor --harness "$h" || fail "$h doctor"
    echo "harness smoke: $h start"
    # 256 MiB of chain reserved, not the 10 GiB default: linux-check's ext4 volume is 4 GiB.
    # `timeout` bounds a harness that keeps retrying. `goalcup start` exits 0 once the run
    # seals, whatever the harness's own exit code.
    run timeout 600 goalcup start --harness "$h" --goal-file "$d/goal/goal.txt" \
        --capture-reserve-bytes 268435456 >"$d/start.out" 2>&1
    rc=$?
    cat "$d/start.out"
    echo "harness smoke: $h start exit $rc"
    [ "$rc" -eq 0 ] || { cat "$d/cfg/daemon.log" 2>/dev/null; fail "$h start exit $rc"; }
    trace=$(sed -n 's/^trace:  *//p' "$d/start.out" | tail -n 1)
    [ -n "$trace" ] && [ -d "$trace" ] || fail "$h no trace folder (the run did not seal)"
    goalcup-verify --capture-integrity "$trace" || fail "$h verify"
    # Only the harness's own name; run this where no other $prog runs (the Docker container).
    if pgrep -x "$prog" >/dev/null; then
        ps -eo pid,ppid,pgid,comm
        fail "$h a $prog process is still running"
    fi
    echo "harness smoke: $h ok"
}

[ $# -gt 0 ] || set -- codex-cli claude-code-cli
for h in "$@"; do smoke "$h"; done
echo "harness smoke: PASS"
