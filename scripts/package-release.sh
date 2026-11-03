#!/bin/sh
# Package one release tarball from binaries already built for <target>:
#
#   cargo build --release --locked --target <target> -p goalcup -p goalcup-verify
#   sh scripts/package-release.sh <target> <version> <outdir>
#
# Writes <outdir>/goalcup-<version>-<target>.tar.gz, laid out as an install prefix, so the
# unpacked directory runs as it is and packaging/install.sh copies it file for file:
#
#   bin/goalcup bin/goalcup-verify
#   LICENSE NOTICE THIRD-PARTY-LICENSES.md
#
# The connectors are embedded in goalcup (PRD-11), so the tarball ships no connectors folder.
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

mkdir -p "$root/bin"
for binary in goalcup goalcup-verify; do
    install -m 0755 "target/$target/release/$binary" "$root/bin/$binary"
done
cp LICENSE NOTICE "$root/"
python3 scripts/third-party-licenses.py --target "$target" >"$root/THIRD-PARTY-LICENSES.md"

mkdir -p "$out"
# No AppleDouble `._` files from macOS tar.
COPYFILE_DISABLE=1 tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
echo "$out/$name.tar.gz"
