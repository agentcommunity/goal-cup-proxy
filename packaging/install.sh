#!/bin/sh
# Install Goal Cup Proxy from a GitHub release. One command, no sudo:
#
#   curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
#
# It picks the build for this machine (macOS or Linux, arm64 or x86_64; WSL2 is Linux), checks the
# download against the SHA256SUMS file of the same release, and installs goalcup and goalcup-verify
# into ~/.local/bin. It writes nothing outside the prefix: no shell profile (when the folder is not
# on PATH it prints one line for the participant to run), nothing of any harness. Run it again to
# upgrade. Options (after `sh -s --`, or as environment variables):
#   --version <x.y.z>   GOALCUP_VERSION   install that release instead of the latest
#   --prefix <dir>      GOALCUP_PREFIX    install under <dir> instead of ~/.local
#   --force             GOALCUP_FORCE=1   install even if a Goal Cup run is recording
# For tests, GOALCUP_RELEASE_BASE (https:// or file://) replaces the GitHub release URL.
#
# Run guard (INSTALL_BLOCKED, INSTALL_UNSURE): see packaging/README.md.
#
# The whole script is one function called on the last line, so a download cut short runs nothing.
set -eu

repo="agentcommunity/goal-cup-proxy"
binaries="goalcup goalcup-verify"

say() { printf '%s\n' "$*"; }
die() {
    printf 'goalcup install: %s\n' "$*" >&2
    exit 1
}

# The Rust target triple of this machine, or a reason it has none.
detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
    arm64 | aarch64) arch=aarch64 ;;
    x86_64 | amd64) arch=x86_64 ;;
    *) die "no build for the CPU '$arch' (the release has arm64 and x86_64)" ;;
    esac
    case "$os" in
    Darwin)
        # A shell started under Rosetta reports x86_64 on an Apple silicon Mac; install the
        # native build.
        if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
            arch=aarch64
        fi
        echo "$arch-apple-darwin"
        ;;
    Linux)
        # The Linux builds link glibc 2.35 or later. musl (Alpine) cannot run them.
        if command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
            die "this Linux uses musl libc; the release is built for glibc (Debian, Ubuntu, Fedora, WSL2)"
        fi
        echo "$arch-unknown-linux-gnu"
        ;;
    *) die "no build for '$os' (the release has macOS and Linux; on Windows use WSL2)" ;;
    esac
}

# fetch <url> <file>: curl, else wget. Fails on an HTTP error. An https:// address is fetched
# over TLS 1.2 or later only, and a redirect may not leave https.
fetch() {
    if command -v curl >/dev/null 2>&1; then
        case "$1" in
        https://*) curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$2" "$1" || die "could not download $1" ;;
        *) curl -fsSL --retry 3 -o "$2" "$1" || die "could not download $1" ;;
        esac
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1" || die "could not download $1"
    else
        die "neither curl nor wget is installed"
    fi
}

# sha256 <file>: the hex digest.
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | sed 's/^.*= //'
    else
        die "no sha256sum, shasum or openssl to check the download with"
    fi
}

# status_mode <file>: the `mode` of the top-level `status` object of `goalcup status --json`, as a
# bare word (a JSON string loses its quotes, a JSON null stays null), or nothing. The participant
# may have no jq or python3, so awk walks the JSON. The `run` object also has a `mode` key and
# may come first, so a plain grep for "mode" could read the wrong one.
status_mode() {
    LC_ALL=C awk '
    { s = s $0 }
    END {
        n = split(s, ch, ""); d = 0; instr = 0; want = 0; top = ""; last = ""; word = ""
        for (i = 1; i <= n; i++) {
            c = ch[i]
            if (instr) {
                if (c == "\\") { i++; tok = tok "?" }
                else if (c == "\"") { instr = 0; last = tok; if (want) { print tok; exit } }
                else tok = tok c
            } else if (want) {
                if (c ~ /[a-z_]/) word = word c
                else if (c == "\"" && word == "") { instr = 1; tok = "" }
                else if (word != "" || (c != " " && c != "\t")) { if (word != "") print word; exit }
            } else if (c == "\"") { instr = 1; tok = "" }
            else if (c == "{" || c == "[") d++
            else if (c == "}" || c == "]") d--
            else if (c == ":") {
                if (d == 1) top = last
                if (d == 2 && top == "status" && last == "mode") want = 1
            }
        }
        if (want && word != "") print word
    }' "$1"
}

# Ask the goalcup that would be replaced whether a run is recording; stop if one is. Called before
# anything in the prefix is touched.
guard_recording_run() {
    [ "$force" != 1 ] || return 0
    existing="$prefix/bin/goalcup"
    if [ ! -f "$existing" ] || [ ! -x "$existing" ]; then
        existing=$(command -v goalcup 2>/dev/null || true)
    fi
    case $existing in
    /*) ;;
    *) return 0 ;; # no goalcup to replace: a first install
    esac

    status_out="$work/status.out"
    status_rc="$work/status.rc"
    status_pid="$work/status.pid"
    # macOS has no `timeout`: run it in the background and kill it after 10 s. The helper writes
    # the exit status to a file when goalcup ends, so no job-control behaviour of the shell is
    # needed. Its own output goes nowhere, so nothing it starts can hold this script's pipes.
    (
        "$existing" status --json >"$status_out" 2>/dev/null </dev/null &
        child=$!
        printf '%s\n' "$child" >"$status_pid"
        code=0
        wait "$child" || code=$?
        printf '%s\n' "$code" >"$status_rc.tmp" && mv "$status_rc.tmp" "$status_rc"
    ) >/dev/null 2>&1 &
    started=$(date +%s)
    while [ ! -f "$status_rc" ]; do
        if [ $(($(date +%s) - started)) -gt 10 ]; then
            if [ -f "$status_pid" ]; then
                kill -9 "$(cat "$status_pid")" 2>/dev/null || true
            fi
            break
        fi
        sleep 0.1 2>/dev/null || sleep 1
    done

    # A killed status leaves no exit code yet, or 137: both read as unsure.
    verdict=unsure
    rc=$(cat "$status_rc" 2>/dev/null || true)
    if [ "$rc" = 0 ]; then
        case $(status_mode "$status_out") in
        recording | losing | writer_slow | null) verdict=blocked ;;
        ?*) verdict=clear ;;
        esac
    elif [ "$rc" = 1 ] && [ ! -s "$status_out" ]; then
        verdict=clear # no run on this computer yet
    fi

    case $verdict in
    blocked)
        printf '%s\n' "INSTALL_BLOCKED: A Goal Cup run is recording on this computer. Update after it ends, or run the installer again with: curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh -s -- --force" >&2
        exit 1
        ;;
    unsure)
        printf '%s\n' "INSTALL_UNSURE: Could not tell whether a Goal Cup run is recording. If one is, stop this update and run it after the run ends." >&2
        ;;
    esac
}

# Restore one promoted binary. The staging files themselves record which renames completed,
# including when a signal arrives between a rename and the next shell command.
restore() {
    destination=$1
    slot=$2
    if [ -e "$slot/old" ] || [ -L "$slot/old" ]; then
        rm -f "$destination" || return 1
        mv "$slot/old" "$destination" || return 1
    elif [ ! -e "$slot/new" ]; then
        rm -f "$destination" || return 1
    fi
}

cleanup() {
    cleanup_status=$1
    trap - EXIT
    trap '' HUP INT TERM
    set +e
    rollback_failed=0
    if [ "$promoting" = 1 ] && [ "$committed" = 0 ]; then
        for binary in $binaries; do
            restore "$prefix/bin/$binary" "$bin_stage/$binary" || rollback_failed=1
        done
    fi
    if [ "$rollback_failed" = 1 ]; then
        printf 'goalcup install: rollback could not finish; recoverable files remain in %s\n' \
            "$bin_stage" >&2
        cleanup_status=1
    else
        [ -z "$bin_stage" ] || rm -rf "$bin_stage"
    fi
    rm -rf "$work"
    exit "$cleanup_status"
}

# The one line that puts the bin folder on PATH now and appends the same export to the profile
# of the participant's shell. Nothing here runs it.
path_line() {
    dir="$prefix/bin"
    case $dir in
    *[\"\'\`\\\$]*) return 1 ;; # a folder name this line cannot quote
    "${HOME:-/nonexistent}"/*) dir="\$HOME${dir#"$HOME"}" ;;
    esac
    case ${SHELL:-} in
    */zsh) profile="~/.zshrc" ;;
    # Terminal on macOS starts bash as a login shell, which reads ~/.bash_profile, not ~/.bashrc.
    */bash) [ "$(uname -s)" = Darwin ] && profile="~/.bash_profile" || profile="~/.bashrc" ;;
    *) profile="~/.profile" ;;
    esac
    export_line="export PATH=\"$dir:\$PATH\""
    say "  $export_line && echo '$export_line' >> $profile"
}

main() {
    version="${GOALCUP_VERSION:-}"
    prefix="${GOALCUP_PREFIX:-}"
    force="${GOALCUP_FORCE:-}"
    while [ $# -gt 0 ]; do
        case "$1" in
        --version | --prefix)
            [ $# -ge 2 ] || die "$1 needs a value"
            if [ "$1" = --version ]; then version=$2; else prefix=$2; fi
            shift 2
            ;;
        --force) force=1 && shift ;;
        *) die "unknown option '$1' (known: --version <x.y.z>, --prefix <dir>, --force)" ;;
        esac
    done
    version=${version#v}
    if [ -z "$prefix" ]; then
        [ -n "${HOME:-}" ] || die "HOME is not set; pass --prefix"
        prefix="$HOME/.local"
    fi

    target=$(detect_target)

    if [ -n "${GOALCUP_RELEASE_BASE:-}" ]; then
        base=${GOALCUP_RELEASE_BASE%/}
    elif [ -n "$version" ]; then
        base="https://github.com/$repo/releases/download/v$version"
    else
        base="https://github.com/$repo/releases/latest/download"
    fi

    work=$(mktemp -d)
    bin_stage=""
    promoting=0
    committed=0
    trap 'cleanup $?' EXIT
    trap 'exit 130' HUP INT TERM

    say "Looking for the $target build ..."
    fetch "$base/SHA256SUMS" "$work/SHA256SUMS"
    # Lines read `<digest>  goalcup-<version>-<target>.tar.gz`; the file name carries the version.
    line=$(grep -E "  goalcup-[0-9][^ ]*-$target\.tar\.gz\$" "$work/SHA256SUMS" | head -n 1 || true)
    [ -n "$line" ] || die "this release has no $target build (see $base/SHA256SUMS)"
    want=${line%% *}
    file=${line##* }
    name=${file%.tar.gz}

    say "Downloading $file ..."
    fetch "$base/$file" "$work/$file"
    got=$(sha256 "$work/$file")
    [ "$got" = "$want" ] || die "checksum mismatch for $file: expected $want, got $got; nothing was installed"

    tar -xzf "$work/$file" -C "$work" || die "could not unpack $file"
    root="$work/$name"

    for binary in $binaries; do
        [ -f "$root/bin/$binary" ] || die "$file does not hold bin/$binary; nothing was installed"
        # Run each new binary where it was unpacked, before anything in the prefix is touched. A
        # build this machine cannot run (a glibc older than the one it was built against, a
        # 32-bit userland) must leave an existing install as it was, and must not report success.
        if ! out=$("$root/bin/$binary" --version 2>&1); then
            hint=""
            if [ "$(uname -s)" = Linux ]; then
                hint="; the Linux builds need glibc 2.35 or later (this system: $(ldd --version 2>&1 | head -n 1))"
            fi
            die "$binary from $file does not run on this machine$hint. Its output: $out. Nothing was installed."
        fi
        # Refuse a conflicting user path before creating anything in the prefix: mv to a
        # directory would put the binary inside it and falsely report a successful install.
        destination="$prefix/bin/$binary"
        if [ -d "$destination" ] || { [ -e "$destination" ] && [ ! -f "$destination" ]; }; then
            die "$destination is not a regular file; nothing was installed"
        fi
    done

    # Last check before the first change: never replace goalcup while a run is recording.
    guard_recording_run

    mkdir -p "$prefix/bin"
    # Stage both binaries beside their destinations before changing any installed file, so each
    # promotion is a rename on one filesystem.
    bin_stage=$(mktemp -d "$prefix/bin/.goalcup-install.XXXXXX")
    for binary in $binaries; do
        mkdir "$bin_stage/$binary"
        cp "$root/bin/$binary" "$bin_stage/$binary/new"
        chmod 755 "$bin_stage/$binary/new"
    done

    # Retain the old files until both promotions and the final executable check succeed. EXIT
    # and handled signals restore them on failure. These renames are not a crash-atomic
    # transaction: SIGKILL or power loss can require manual recovery.
    promoting=1
    for binary in $binaries; do
        if [ -e "$prefix/bin/$binary" ] || [ -L "$prefix/bin/$binary" ]; then
            mv "$prefix/bin/$binary" "$bin_stage/$binary/old"
        fi
        mv "$bin_stage/$binary/new" "$prefix/bin/$binary"
    done
    installed_version=$("$prefix/bin/goalcup" --version)
    committed=1

    # Show the folder the way a person types it: ~ for the home folder.
    where="$prefix/bin"
    case $where in
    "${HOME:-/nonexistent}"/*) where="~${where#"$HOME"}" ;;
    esac
    say "Installed $installed_version in $where."
    found=$(command -v goalcup 2>/dev/null || true)
    if [ -n "$found" ] && [ "$found" != "$prefix/bin/goalcup" ]; then
        say "Note: your PATH finds $found first, not the one just installed."
    fi
    case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *)
        say ""
        say "$where is not on your PATH. To add it now and for new shells, run:"
        path_line || say "  (add $prefix/bin to PATH in your shell profile)"
        ;;
    esac
    say ""
    say "Next: goalcup login"
}

main "$@"
