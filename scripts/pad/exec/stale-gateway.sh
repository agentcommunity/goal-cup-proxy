#!/usr/bin/env bash
# Pre script: starts the harness's daemon as a previous start would have left it: isolated home, none of our env.
# PRD-11 acceptance 5 (PRD-12 section 3.1); the daemon is left running for goalcup start to meet.
set -eu
[ -n "${PAD_DAEMON:-}" ] || { echo "no daemon for this harness"; exit 0; }
vars=$(python3 -c 'import json, shlex, sys; hv = json.load(open(sys.argv[1])).get("home_vars") or {"HOME": "{home}"}
print(" ".join(shlex.quote(k + "=" + v.replace("{home}", sys.argv[2])) for k, v in hv.items()))' "$PAD_RUN/settings.json" "$PAD_HOME")
eval "set -- $vars"  # the connector's `home` mapping, as settings.json carries it
env -i PATH="$PATH" TERM="${TERM:-xterm}" HOME="$HOME" "$@" setsid nohup sh -c "exec $PAD_DAEMON" </dev/null >/dev/null 2>&1 &
pid=$!
sleep 1
kill -0 "$pid" 2>/dev/null || { echo "'$PAD_DAEMON' exited at once"; exit 1; }
echo "started '$PAD_DAEMON' with $*: pid $pid running"
