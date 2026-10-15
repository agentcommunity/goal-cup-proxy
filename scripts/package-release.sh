#!/bin/sh
# Package one release tarball from binaries already built for <target>:
#
#   cargo build --release --locked --target <target> -p goalcup-cli -p goalcup-daemon -p goalcup-verifier
#   sh scripts/package-release.sh <target> <version> <outdir>
#
# Writes <outdir>/goalcup-<version>-<target>.tar.gz, laid out as an install prefix, so the
# unpacked directory runs as it is and Homebrew installs it file for file:
#
#   bin/goalcup bin/goalcup-daemon bin/goalcup-verify
#   share/goalcup/connectors/   goalcup finds it as <exe dir>/../share/goalcup/connectors;
#                               every folder under connectors/, with no list of names, so a
#                               merged harness ships by being merged
#   LICENSE NOTICE THIRD-PARTY-LICENSES.md
#
# The connectors are the committed tree (`git archive HEAD`), never a working copy: their pins
# cover every byte, so a stray local file would make every start refuse.
set -eu

if [ $# -ne 3 ]; then
    echo "usage: sh scripts/package-release.sh <target> <version> <outdir>" >&2
    exit 2
fi
target=$1
version=$2
out=$3

cd "$(dirname "$0")/.."
name="goalcup-$version-$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root="$stage/$name"

mkdir -p "$root/bin" "$root/share/goalcup"
for binary in goalcup goalcup-daemon goalcup-verify; do
    install -m 0755 "target/$target/release/$binary" "$root/bin/$binary"
done
git archive HEAD connectors | tar -x -C "$root/share/goalcup"
cp LICENSE NOTICE "$root/"
python3 scripts/third-party-licenses.py --target "$target" >"$root/THIRD-PARTY-LICENSES.md"

mkdir -p "$out"
# No AppleDouble `._` files from macOS tar.
COPYFILE_DISABLE=1 tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
echo "$out/$name.tar.gz"
