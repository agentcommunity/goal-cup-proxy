#!/bin/sh
# Cut a release. The workspace `version` in Cargo.toml is the only place a version is written.
# Every crate inherits it, and the binaries and report fields read it at build time. `main` is
# protected, so this script never pushes it: a version bump is a pull request, and the release
# is a tag on what that pull request merged.
#
#   sh scripts/release.sh prepare <x.y.z>   branch release/v<x.y.z> from origin/main, set the
#                                           version, update Cargo.lock, run the version check,
#                                           commit, push the branch and open a pull request.
#                                           If origin/main already has <x.y.z>, it says so and
#                                           does nothing.
#   sh scripts/release.sh tag <x.y.z>       after that pull request is merged: fetch, check that
#                                           origin/main's version is <x.y.z>, tag origin/main's
#                                           commit (not any local branch) and push only the tag.
#
# The tag starts .github/workflows/release.yml, which builds the four targets and publishes the
# release. It refuses a tag that differs from Cargo.toml.
set -eu

usage() {
    echo "usage: sh scripts/release.sh prepare <x.y.z>" >&2
    echo "       sh scripts/release.sh tag <x.y.z>" >&2
    exit 2
}

[ $# -eq 2 ] || usage
command=$1
new=$2
case "$command" in
prepare | tag) ;;
*) usage ;;
esac
if ! printf '%s\n' "$new" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    echo "release: '$new' is not a version of the form x.y.z" >&2
    exit 2
fi

cd "$(dirname "$0")/.."

die() {
    echo "release: $*" >&2
    exit 1
}

# The workspace version in a Cargo.toml read from stdin.
version_of() {
    sed -n 's/^version = "\(.*\)"$/\1/p' | head -n 1
}

git fetch -q origin main --tags || die "could not fetch origin"
main_version=$(git show origin/main:Cargo.toml | version_of)
[ -n "$main_version" ] || die "no 'version = \"...\"' line in origin/main's Cargo.toml"

if [ "$command" = tag ]; then
    [ "$main_version" = "$new" ] ||
        die "origin/main is at version $main_version, not $new. Run 'prepare $new' and merge its pull request first."
    if git rev-parse -q --verify "refs/tags/v$new" >/dev/null ||
        [ -n "$(git ls-remote --tags origin "refs/tags/v$new")" ]; then
        die "the tag v$new already exists"
    fi
    git tag -a "v$new" -m "goalcup $new" origin/main
    if ! git push origin "refs/tags/v$new"; then
        git tag -d "v$new" >/dev/null
        die "could not push the tag v$new; the local tag was removed"
    fi
    echo "release: tagged origin/main ($(git rev-parse --short origin/main)) as v$new and pushed the tag."
    echo "The release workflow is running: gh run list --workflow release.yml"
    exit 0
fi

# prepare
if [ "$main_version" = "$new" ]; then
    echo "release: origin/main is already at version $new, so there is nothing to bump."
    echo "Next: sh scripts/release.sh tag $new"
    exit 0
fi
highest=$(printf '%s\n%s\n' "$main_version" "$new" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)
[ "$highest" = "$new" ] || die "$new is not higher than origin/main's version $main_version"
[ -z "$(git status --porcelain)" ] || die "the working tree is not clean"
branch="release/v$new"
if git rev-parse -q --verify "refs/heads/$branch" >/dev/null ||
    [ -n "$(git ls-remote --heads origin "$branch")" ]; then
    die "the branch $branch already exists"
fi

# Work on a detached origin/main, so a failed check leaves no branch behind.
start=$(git symbolic-ref --short -q HEAD || git rev-parse HEAD)
git switch -q --detach origin/main
restore() {
    git checkout -q -- Cargo.toml Cargo.lock
    git checkout -q "$start"
}
# The first `version = "..."` line is the workspace's (under [workspace.package]).
awk -v v="$new" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' \
    Cargo.toml >Cargo.toml.new
mv Cargo.toml.new Cargo.toml
if ! cargo update --workspace --offline; then
    restore
    die "cargo could not update Cargo.lock"
fi
if ! python3 scripts/check-version.py; then
    restore
    die "the version check fails with $new; nothing was committed"
fi
git switch -q -c "$branch"
git add Cargo.toml Cargo.lock
git commit -q -m "Release $new"
if ! { git push -q -u origin "$branch" &&
    gh pr create --base main --head "$branch" --title "Release $new" \
        --body "Sets the workspace version to $new. After it is merged: sh scripts/release.sh tag $new"; }; then
    git checkout -q "$start"
    die "the release commit is on the local branch $branch (and on origin if the push worked). Finish by hand: git push -u origin $branch && gh pr create --base main --head $branch"
fi
git checkout -q "$start"
echo "release: pull request opened for $branch. After it is merged: sh scripts/release.sh tag $new"
