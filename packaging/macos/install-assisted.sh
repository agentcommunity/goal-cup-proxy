#!/bin/sh
# Thin public entry point for the descriptor-rooted assisted macOS installer.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
exec python3 "$SCRIPT_DIR/install_assisted.py" "$@"
