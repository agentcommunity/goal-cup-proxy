#!/bin/sh
# Assisted uninstall of a bundle `install-assisted.sh` installed: unload the launchd USER
# agent, remove the installed program and connector files, and remove the daemon config —
# but PRESERVE `<prefix>/var/artifacts` and `<prefix>/var/workspaces`, because those hold
# whatever entries a participant actually ran. Uninstalling is never how evidence is lost.
#
# Usage:
#   sh uninstall-assisted.sh --prefix <installed-prefix> [--label <launchd-label>]
#
# Refuses (rather than guessing) a prefix that was not installed by install-assisted.sh: it
# requires and reads back `<prefix>/.goalcup-install-manifest.json`, and refuses a --label
# that does not match what that manifest recorded.

set -eu

refuse() {
    echo "uninstall-assisted.sh: $1" >&2
    exit 1
}

prefix=""
label=""

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || refuse "--prefix requires a value"
            prefix="$2"
            shift 2
            ;;
        --label)
            [ $# -ge 2 ] || refuse "--label requires a value"
            label="$2"
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

recorded_label=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['label'])" "$manifest")
recorded_plist=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['plist_path'])" "$manifest")

if [ -n "$label" ] && [ "$label" != "$recorded_label" ]; then
    refuse "--label $label does not match the installed label $recorded_label recorded in $manifest"
fi
label="$recorded_label"

echo "uninstall-assisted.sh: unloading launchd label $label"
launchctl bootout "gui/$me/$label" 2>/dev/null || true

if [ -e "$recorded_plist" ] || [ -L "$recorded_plist" ]; then
    rm -f "$recorded_plist"
fi

# Also remove the canonical plist path for this label in case the manifest's recorded path
# ever drifts from the platform default — belt and braces, never a second guess about what
# else to delete.
default_plist="$HOME/Library/LaunchAgents/$label.plist"
if [ -e "$default_plist" ] || [ -L "$default_plist" ]; then
    rm -f "$default_plist"
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
