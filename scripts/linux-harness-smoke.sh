#!/bin/sh
# No-login smoke of the measured Linux harness builds (x86_64 or arm64, from `uname -m`): Codex
# 0.153.0 and Claude Code 2.1.283, the builds docs/harnesses/ describes.
# For each harness: download its npm platform package, check the package's and the binary's
# SHA-256, put the binary first on PATH, then `goalcup doctor` must pass and one `goalcup start`
# with a never-logged-in home must launch it, stop at auth, seal and verify (exit 0), and leave
# no process named codex or claude running (so run it where no other one runs, as
# linux-check.sh's container does). Nothing here signs in; no model reply is expected.
# `goalcup start` is driven the way the connector's manifest says: a headless connector gets
# `--goal-file`; a chat-mode connector (`[capture] interactive = true`) gets none and a terminal
# of its own, where the smoke types one prompt, waits, then hangs the terminal up, as a
# participant closing the window does (the run still seals).
# Needs: Linux, goalcup, goalcup-daemon and goalcup-verify on PATH, GOALCUP_CONNECTORS_DIR set
# when the binaries are not beside a connectors/ directory, TMPDIR on ext4 or XFS, curl,
# sha256sum, pgrep, python3 (3.11 or newer: tomllib, pty), and network (the npm registry, and `goalcup start` checks the provider host
# answers). `sh scripts/linux-check.sh` runs it in Docker when LINUX_CHECK_HARNESSES=1.
# Meant for CI or a container: the leftover-process check sees any process of that name.
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

# The npm tarball and the binary inside it, with both SHA-256s, as measured in docs/harnesses/.
# The connectors record the build that runs rather than pin one (D53), so these checks are what
# makes this smoke run exactly that build.
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

# The connectors directory goalcup reads (config.rs `connectors_dir`): the variable, else
# `../share/goalcup/connectors` or `../../connectors` beside the goalcup binary.
connectors_dir() {
    if [ -n "${GOALCUP_CONNECTORS_DIR:-}" ]; then
        echo "$GOALCUP_CONNECTORS_DIR"
        return
    fi
    bindir=$(dirname "$(command -v goalcup)")
    for c in "$bindir/../share/goalcup/connectors" "$bindir/../../connectors"; do
        if [ -d "$c" ]; then
            echo "$c"
            return
        fi
    done
}

# Prints `chat` for a connector whose manifest says `[capture] interactive = true`, else `headless`.
mode_of() {
    python3 - "$(connectors_dir)/$1/harness.toml" <<'PY'
import sys, tomllib

with open(sys.argv[1], "rb") as f:
    manifest = tomllib.load(f)
print("chat" if manifest.get("capture", {}).get("interactive", False) else "headless")
PY
}

# Runs `goalcup start` of a chat-mode connector on a terminal of its own: waits for the launch
# line, types one prompt, lets the harness answer, then hangs up the terminal's foreground group
# (the harness's and goalcup's own) and waits for goalcup to seal and exit. Exits with goalcup's
# exit code. Usage: chat_driver.py <transcript file> <command...>
write_chat_driver() {
    cat >"$work/chat_driver.py" <<'PY'
import fcntl, os, pty, re, select, signal, struct, sys, termios, time

transcript, argv = sys.argv[1], sys.argv[2:]
os.environ["TERM"] = "xterm-256color"
pid, fd = pty.fork()
if pid == 0:
    os.execvp(argv[0], argv)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
log = open(transcript, "wb")
seen = bytearray()
last_byte = time.monotonic()
open_ = True


def pump(wait):
    # Read what the terminal shows; answer the queries a TUI makes of its terminal.
    global last_byte, open_
    ready, _, _ = select.select([fd], [], [], wait)
    if not ready:
        return
    try:
        chunk = os.read(fd, 65536)
    except OSError:
        chunk = b""
    if not chunk:
        open_ = False
        return
    seen.extend(chunk)
    log.write(chunk)
    log.flush()
    last_byte = time.monotonic()
    for _ in range(chunk.count(b"\x1b[6n")):
        os.write(fd, b"\x1b[1;1R")
    for _ in range(len(re.findall(rb"\x1b\[0?c", chunk))):
        os.write(fd, b"\x1b[?1;2c")


def settle(minimum, maximum):
    # At least `minimum` seconds, then until the terminal has been quiet for 3 s (or `maximum`).
    start = time.monotonic()
    while open_ and time.monotonic() - start < maximum:
        pump(0.25)
        if time.monotonic() - start >= minimum and time.monotonic() - last_byte >= 3:
            return


def wait_for(text, seconds):
    deadline = time.monotonic() + seconds
    while open_ and text not in seen and time.monotonic() < deadline:
        pump(0.25)
    return text in seen


def kill_group(sig):
    try:
        os.killpg(pid, sig)
    except OSError:
        pass


def finish(code):
    tail = re.sub(rb"\x1b(\[[0-9;?<>=]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[@-Z\\-_])", b"", bytes(seen))
    lines = [l.strip() for l in tail.decode("utf-8", "replace").replace("\r", "\n").split("\n")]
    print("chat driver: the terminal showed, last lines:")
    for line in [l for l in lines if l][-25:]:
        print("  | " + line[:160])
    sys.exit(code)


if not wait_for(b"Recording in chat mode", 180):
    print("chat driver: no launch line")
    kill_group(signal.SIGKILL)
    finish(97)
print("chat driver: launched; typing one prompt")
settle(5, 60)
os.write(fd, b"Print the word hello.\r")
settle(10, 60)
print("chat driver: hanging up the terminal")
try:
    os.killpg(os.tcgetpgrp(fd), signal.SIGHUP)
except OSError:
    os.kill(pid, signal.SIGHUP)
deadline = time.monotonic() + 180
status = None
while time.monotonic() < deadline:
    pump(0.25)
    done, st = os.waitpid(pid, os.WNOHANG)
    if done:
        status = st
        break
if status is None:
    print("chat driver: goalcup did not exit after the hangup")
    kill_group(signal.SIGKILL)
    finish(98)
while open_:
    pump(0.1)
    if time.monotonic() - last_byte > 1:
        break
finish(os.waitstatus_to_exitcode(status))
PY
}

smoke() {
    h=$1
    package "$h"
    d=$work/$h
    mkdir -p "$d/pkg" "$d/bin" "$d/home" "$d/goal" || fail "$h mkdir"
    echo "harness smoke: $h ($arch) fetch"
    curl -fsSL --retry 3 --max-time 600 -o "$d/pkg.tgz" "$url" || fail "$h download"
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
    echo "harness smoke: $h version: $(run "$prog" --version 2>/dev/null | head -n 1)"
    run goalcup doctor --harness "$h" || fail "$h doctor"
    mode=$(mode_of "$h") || fail "$h manifest ($(connectors_dir)/$h/harness.toml)"
    echo "harness smoke: $h start ($mode)"
    # The run holds its 3 GiB reservation and wants 1 GiB more free (linux-check gives the ext4
    # volume 8 GiB for this). `timeout` bounds a harness that keeps retrying. `goalcup start`
    # exits 0 once the run seals, whatever the harness's own exit code.
    case $mode in
    chat)
        write_chat_driver
        # The run's workspace is made in the current directory, which must not be inside a Git
        # worktree (a CI checkout is one), so start from the empty goal directory.
        (
            cd "$d/goal" &&
                run timeout 900 python3 "$work/chat_driver.py" "$d/tty.out" \
                    goalcup start --harness "$h" >"$d/start.out" 2>&1
        )
        ;;
    *)
        echo "Print the word hello." >"$d/goal/goal.txt"
        run timeout 600 goalcup start --harness "$h" --goal-file "$d/goal/goal.txt" \
            >"$d/start.out" 2>&1
        ;;
    esac
    rc=$?
    cat "$d/start.out"
    echo "harness smoke: $h start exit $rc"
    [ "$rc" -eq 0 ] || {
        cat "$d/cfg/daemon.log" "$d"/cfg/runs/*/host.log 2>/dev/null
        fail "$h start exit $rc"
    }
    # The run's own record names the trace folder: <cfg>/runs/<run_id>/run.json.
    rec=$(ls "$d"/cfg/runs/*/run.json 2>/dev/null | tail -n 1)
    [ -n "$rec" ] || fail "$h no run record (run.json) under $d/cfg/runs"
    trace=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["entries"][-1]["trace_dir"])' "$rec") ||
        fail "$h the run record $rec could not be read"
    [ -n "$trace" ] && [ -d "$trace" ] || fail "$h the trace folder named in $rec was not found"
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
