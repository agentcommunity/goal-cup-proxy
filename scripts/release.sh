#!/bin/sh
# Cut a release locally: set the one version, update Cargo.lock, commit, and tag.
#
#   sh scripts/release.sh <x.y.z>
#
# The workspace `version` in Cargo.toml is the only place a version is written. Every crate
# inherits it, and the binaries and report fields read it at build time. This script changes that
# line, lets cargo update the lock file, commits both as "Release <x.y.z>" and tags the commit
# `v<x.y.z>`. It pushes nothing. It refuses to run unless
#
#   - the branch is main and the working tree is clean,
#   - <x.y.z> is a plain three-part version higher than the current one, and
#   - the tag does not exist yet.
#
# Then push the commit and the tag by hand; the tag starts .github/workflows/release.yml, which
# refuses a tag that differs from Cargo.toml:
#
#   git push origin main v<x.y.z>
set -eu

if [ $# -ne 1 ]; then
    echo "usage: sh scripts/release.sh <x.y.z>" >&2
    exit 2
fi
new=$1
if ! printf '%s\n' "$new" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    echo "release: '$new' is not a version of the form x.y.z" >&2
    exit 2
fi

cd "$(dirname "$0")/.."

branch=$(git symbolic-ref --short -q HEAD || true)
if [ "$branch" != main ]; then
    echo "release: run this on main (the branch is '${branch:-detached}')" >&2
    exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
    echo "release: the working tree is not clean" >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/v$new" >/dev/null; then
    echo "release: the tag v$new already exists" >&2
    exit 1
fi

old=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
if [ -z "$old" ]; then
    echo "release: no 'version = \"...\"' line in Cargo.toml" >&2
    exit 1
fi
highest=$(printf '%s\n%s\n' "$old" "$new" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)
if [ "$old" = "$new" ] || [ "$highest" != "$new" ]; then
    echo "release: $new is not higher than the current version $old" >&2
    exit 1
fi

# The first `version = "..."` line is the workspace's (under [workspace.package]).
awk -v v="$new" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' \
    Cargo.toml >Cargo.toml.new
mv Cargo.toml.new Cargo.toml
cargo update --workspace --offline

git add Cargo.toml Cargo.lock
git commit -q -m "Release $new"
git tag -a "v$new" -m "goalcup $new"
echo "release: $old -> $new committed and tagged v$new. Nothing is pushed. To publish:"
echo "  git push origin main v$new"
