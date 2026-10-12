#!/bin/sh
# Assisted uninstall of a bundle `install-assisted.sh` installed: remove the installed program
# and connector files and the daemon config, but PRESERVE `<prefix>/var/artifacts` and
# `<prefix>/var/workspaces`, because those hold whatever entries a participant actually ran.
# Uninstalling is never how evidence is lost. There is no service to unload: the daemon is
# started by `goalcup start` for a run and exits once nothing is unsealed.
#
# Usage:
#   sh uninstall-assisted.sh --prefix <installed-prefix>
#
# Refuses (rather than guessing) a prefix that was not installed by install-assisted.sh: it
# requires `<prefix>/.goalcup-install-manifest.json`. An install made by an earlier installer
# also loaded a launchd agent; its manifest names the label, and this script prints the one
# command that removes it rather than running launchctl itself.

set -eu

refuse() {
    echo "uninstall-assisted.sh: $1" >&2
    exit 1
}

prefix=""

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || refuse "--prefix requires a value"
            prefix="$2"
            shift 2
            ;;
        *)
            refuse "unknown argument: $1"
            ;;
    esac
done

[ -n "$prefix" ] || refuse "--prefix is required"
[ "$(uname -s)" = "Darwin" ] || refuse "this script only uninstalls the macOS assisted bundle (uname -s != Darwin)"

[ -e "$prefix" ] || refuse "nothing installed at: $prefix"
[ ! -L "$prefix" ] || refuse "refusing a symlinked prefix path: $prefix"
[ -d "$prefix" ] || refuse "not a directory: $prefix"
prefix=$(CDPATH= cd -- "$prefix" && pwd -P)

me=$(id -u)
owner=$(stat -f '%u' "$prefix")
[ "$owner" = "$me" ] || refuse "refusing a prefix not owned by the current user: $prefix"

manifest="$prefix/.goalcup-install-manifest.json"
[ -f "$manifest" ] || refuse "no install manifest at $manifest — refusing to guess what to remove (this prefix was not installed by install-assisted.sh, or was already uninstalled)"
[ ! -L "$manifest" ] || refuse "refusing a symlinked install manifest: $manifest"

# An earlier installer's manifest (format 1) records the launchd agent it loaded.
recorded_label=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('label', ''))" "$manifest")
recorded_plist=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('plist_path', ''))" "$manifest")
if [ -n "$recorded_label" ]; then
    echo "uninstall-assisted.sh: this prefix was installed with the launchd agent $recorded_label;" \
        "this script no longer touches launchd. Remove it yourself with" \
        "launchctl bootout gui/$me/$recorded_label && rm -f '$recorded_plist'" >&2
fi

rm -rf "${prefix:?}/bin"
rm -rf "${prefix:?}/share"
rm -f "$prefix/etc/daemon.toml"
rmdir "$prefix/etc" 2>/dev/null || true
rm -f "$manifest"

echo "uninstall-assisted.sh: removed program, connector and config files under $prefix"
if [ -d "$prefix/var/artifacts" ] || [ -d "$prefix/var/workspaces" ]; then
    echo "uninstall-assisted.sh: preserved entry artifacts at $prefix/var/artifacts and $prefix/var/workspaces"
else
    echo "uninstall-assisted.sh: no artifacts were ever written under $prefix/var"
fi
