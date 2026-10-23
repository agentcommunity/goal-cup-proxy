#!/bin/sh
# Install Goal Cup Proxy from a GitHub release. One command, no sudo:
#
#   curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
#
# It picks the build for this machine (macOS or Linux, arm64 or x86_64; WSL2 is Linux), checks the
# download against the SHA256SUMS file of the same release, and installs into ~/.local:
#
#   ~/.local/bin/goalcup  goalcup-daemon  goalcup-verify
#   ~/.local/share/goalcup/connectors/   (goalcup finds it as <exe dir>/../share/goalcup/connectors)
#
# A download by curl is not quarantined by macOS, so the unsigned binaries run. The script writes
# nothing outside the prefix: it does not edit a shell profile and never touches ~/.claude,
# ~/.codex or any other harness configuration. Run it again to upgrade.
#
# Options (as arguments after `sh -s --`, or as environment variables):
#   --version <x.y.z>   GOALCUP_VERSION   install that release instead of the latest
#   --prefix <dir>      GOALCUP_PREFIX    install under <dir> instead of ~/.local
# For tests: GOALCUP_RELEASE_BASE is a URL (https:// or file://) that holds SHA256SUMS and the
# tarballs; it replaces the GitHub release URL.
#
# The whole script is one function called on the last line, so a download cut short runs nothing.
set -eu

repo="agentcommunity/goal-cup-proxy"

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

# Restore one promoted destination. The staging files themselves record which renames
# completed, including when a signal arrives between a rename and the next shell command.
restore() {
    destination=$1
    slot=$2
    kind=$3
    if [ -e "$slot/old" ] || [ -L "$slot/old" ]; then
        if [ "$kind" = file ]; then
            rm -f "$destination" || return 1
        else
            rm -rf "$destination" || return 1
        fi
        mv "$slot/old" "$destination" || return 1
    elif [ ! -e "$slot/new" ]; then
        if [ "$kind" = file ]; then
            rm -f "$destination" || return 1
        else
            rm -rf "$destination" || return 1
        fi
    fi
}

cleanup() {
    cleanup_status=$1
    trap - EXIT
    trap '' HUP INT TERM
    set +e
    rollback_failed=0
    if [ "$promoting" = 1 ] && [ "$committed" = 0 ]; then
        for binary in goalcup-verify goalcup-daemon goalcup; do
            restore "$prefix/bin/$binary" "$bin_stage/$binary" file || rollback_failed=1
        done
        restore "$prefix/share/goalcup" "$share_stage" directory || rollback_failed=1
    fi
    if [ "$rollback_failed" = 1 ]; then
        printf 'goalcup install: rollback could not finish; recoverable files remain in %s and %s\n' \
            "$bin_stage" "$share_stage" >&2
        cleanup_status=1
    else
        [ -z "$bin_stage" ] || rm -rf "$bin_stage"
        [ -z "$share_stage" ] || rm -rf "$share_stage"
    fi
    rm -rf "$work"
    exit "$cleanup_status"
}

main() {
    version="${GOALCUP_VERSION:-}"
    prefix="${GOALCUP_PREFIX:-}"
    while [ $# -gt 0 ]; do
        case "$1" in
        --version)
            [ $# -ge 2 ] || die "--version needs a value"
            version=$2
            shift 2
            ;;
        --prefix)
            [ $# -ge 2 ] || die "--prefix needs a value"
            prefix=$2
            shift 2
            ;;
        *) die "unknown option '$1' (known: --version <x.y.z>, --prefix <dir>)" ;;
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
    share_stage=""
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
    [ -x "$root/bin/goalcup" ] && [ -d "$root/share/goalcup/connectors" ] ||
        die "$file does not hold bin/goalcup and share/goalcup/connectors; nothing was installed"

    # Run each new binary where it was unpacked, before anything in the prefix is touched. A
    # build this machine cannot run (a glibc older than the one it was built against, a 32-bit
    # userland) must leave an existing install as it was, and must not report success.
    for binary in goalcup goalcup-daemon goalcup-verify; do
        if ! out=$("$root/bin/$binary" --version 2>&1); then
            hint=""
            if [ "$(uname -s)" = Linux ]; then
                hint="; the Linux builds need glibc 2.35 or later (this system: $(ldd --version 2>&1 | head -n 1))"
            fi
            die "$binary from $file does not run on this machine$hint. Its output: $out. Nothing was installed."
        fi
    done

    # Refuse conflicting user paths before creating anything in the prefix. In particular,
    # mv to a directory would put a binary inside it and falsely report a successful install.
    for binary in goalcup goalcup-daemon goalcup-verify; do
        destination="$prefix/bin/$binary"
        if [ -d "$destination" ] || { [ -e "$destination" ] && [ ! -f "$destination" ]; }; then
            die "$destination is not a regular file; nothing was installed"
        fi
    done
    share="$prefix/share/goalcup"
    if { [ -e "$share" ] || [ -L "$share" ]; } && [ ! -d "$share" ]; then
        die "$share is not a directory; nothing was installed"
    fi

    mkdir -p "$prefix/bin" "$prefix/share"
    # Stage every artifact before changing any installed file. Separate private directories
    # keep each promotion on the destination filesystem, even when bin and share differ.
    bin_stage=$(mktemp -d "$prefix/bin/.goalcup-install.XXXXXX")
    share_stage=$(mktemp -d "$prefix/share/.goalcup-install.XXXXXX")
    cp -R "$root/share/goalcup" "$share_stage/new"
    cp "$root/LICENSE" "$root/NOTICE" "$root/THIRD-PARTY-LICENSES.md" "$share_stage/new/"
    for binary in goalcup goalcup-daemon goalcup-verify; do
        mkdir "$bin_stage/$binary"
        cp "$root/bin/$binary" "$bin_stage/$binary/new"
        chmod 755 "$bin_stage/$binary/new"
    done

    # Retain the old files until all promotions and the final executable check succeed.
    # EXIT and handled signals restore them on failure. These individual renames are not a
    # crash-atomic transaction: SIGKILL or power loss can require manual recovery.
    promoting=1
    if [ -e "$share" ] || [ -L "$share" ]; then
        mv "$share" "$share_stage/old"
    fi
    mv "$share_stage/new" "$share"
    for binary in goalcup goalcup-daemon goalcup-verify; do
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
        say ""
        say "Note: your PATH finds $found first, not the one just installed. Remove it or put"
        say "$prefix/bin before it on PATH."
    fi
    case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *)
        say ""
        # The folder as the guide spells it (~/.local/bin); the export line keeps the full path.
        say "$where is not on your PATH. Add it for this shell with:"
        say "  export PATH=\"$prefix/bin:\$PATH\""
        say "and put the same line in your shell profile (~/.zshrc or ~/.bashrc) to keep it."
        ;;
    esac
    say ""
    say "Next: goalcup login"
}

main "$@"
