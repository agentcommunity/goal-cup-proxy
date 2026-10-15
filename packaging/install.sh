#!/bin/sh
# Install the Goal Cup tracer from a GitHub release. One command, no sudo:
#
#   curl -fsSL https://github.com/agentcommunity/goal-cup-tracer/releases/latest/download/install.sh | sh
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

repo="agentcommunity/goal-cup-tracer"

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

# fetch <url> <file>: curl, else wget. Fails on an HTTP error.
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 3 -o "$2" "$1" || die "could not download $1"
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
    trap 'rm -rf "$work"' EXIT HUP INT TERM

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

    mkdir -p "$prefix/bin" "$prefix/share"
    # The connectors first and whole: each pin covers every byte of its folder, so the folder is
    # replaced, never merged into an older one. A rename in the same directory is atomic.
    share="$prefix/share/goalcup"
    cp -R "$root/share/goalcup" "$prefix/share/goalcup.new.$$"
    cp "$root/LICENSE" "$root/NOTICE" "$root/THIRD-PARTY-LICENSES.md" "$prefix/share/goalcup.new.$$/" 2>/dev/null || true
    if [ -e "$share" ]; then
        mv "$share" "$prefix/share/goalcup.old.$$"
    fi
    mv "$prefix/share/goalcup.new.$$" "$share"
    rm -rf "$prefix/share/goalcup.old.$$"
    # Each binary is copied beside its place and renamed over it, so a goalcup that is running now
    # keeps its old file and no copy is ever half written.
    for binary in goalcup goalcup-daemon goalcup-verify; do
        cp "$root/bin/$binary" "$prefix/bin/.$binary.new.$$"
        chmod 755 "$prefix/bin/.$binary.new.$$"
        mv -f "$prefix/bin/.$binary.new.$$" "$prefix/bin/$binary"
    done

    say "Installed $("$prefix/bin/goalcup" --version) into $prefix"
    case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *)
        say ""
        say "$prefix/bin is not on your PATH. Add it for this shell with:"
        say "  export PATH=\"$prefix/bin:\$PATH\""
        say "and put the same line in your shell profile (~/.zshrc or ~/.bashrc) to keep it."
        ;;
    esac
    say ""
    say "Next: goalcup doctor"
}

main "$@"
