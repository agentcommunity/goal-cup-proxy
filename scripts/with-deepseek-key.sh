#!/bin/sh -p
# The owner's key loader for the Hermes reference row (seams design v2 section 5, hardened by the
# lane B secret review I-3 and rereview R-1). It holds no value. Run it as a script file, never
# pasted into an interactive shell, and in privileged mode:
#
#   /bin/sh -p scripts/with-deepseek-key.sh ENV_FILE start --harness hermes-cli \
#       [--practice] --goal-file GOAL
#
# (Executing the file does the same: the shebang line above says -p. Invoked without -p, the
# script runs itself again under `/bin/sh -p` before it does anything else.)
#
# `-p` is what keeps a hostile environment out of the shell: privileged mode skips `BASH_ENV`
# and `ENV`, does not import exported shell functions (a `BASH_FUNC_sed%%` that would print what
# `sed` is handed) and does not honour an inherited `SHELLOPTS`. On top of that:
#   * tracing is turned off first (`set +x`), so `sh -x` cannot print the key;
#   * the key is read with shell builtins only, and nothing external runs between reading it
#     and the `exec`: there is no command left for a replaced `PATH` entry to stand in for;
#   * only the ONE `DEEPSEEK_API_KEY=` line of ENV_FILE is read, which may hold any other
#     variables: the file is never sourced and nothing else from it is exported;
#   * it refuses unless ENV_FILE is mode 0600 and holds exactly one such line, whose value is
#     20 to 256 characters of [A-Za-z0-9_-], bare or in double quotes (no CR, no single
#     quotes, no trailing comment, no `export`);
#   * it executes exactly `goalcup start --harness hermes-cli` with `--goal-file` and, when asked,
#     `--practice` (a practice run: `scripts/matrix.sh` uses it, because staging refuses a
#     competition start before its dry-run window), and refuses any other command, harness or
#     option: the key is for the Hermes row and for nothing else it could be handed to;
#   * it never prints the value, in a message or anywhere.
#
# What this does not defend against is a same-user process reading the environment of `goalcup`
# or of Hermes once they run (D37), and a `goalcup` earlier on PATH than the one meant.
set +x
# Invoked without -p (`sh script`, `bash script`): run again privileged, before anything else
# does. `case` is a keyword and `exec` a special builtin, so no exported function can stand in for
# either, and the key has not been read yet.
case $- in *p*) ;; *) exec /bin/sh -p -- "$0" "$@" ;; esac
set +o xtrace 2>/dev/null || true
unset SHELLOPTS BASH_ENV ENV 2>/dev/null || true
set -eu

# A refusal is exit 1, the CLI's own "refused, and you can fix it" rung (`goalcup-cli` lib.rs). Exit 2
# is reserved there for a `Fail` verdict relayed from the verifier, which a caller branching on
# the number would otherwise read a refused loader as.
refuse() {
  echo "refused: $1" >&2
  exit 1
}

# Only the fixed target, and only these options after it (checked before the file is opened).
# `--practice` takes no value, and `--practice=...` is not it.
check_rest() {
  while [ $# -gt 0 ]; do
    case $1 in
      --goal-file)
        [ $# -ge 2 ] || refuse "$1 needs a value"
        shift 2
        ;;
      --goal-file=*) shift ;;
      --practice) shift ;;
      *) refuse "only --goal-file and --practice may follow \`start --harness hermes-cli\`" ;;
    esac
  done
}

[ $# -ge 4 ] || refuse "usage: /bin/sh -p with-deepseek-key.sh ENV_FILE start --harness hermes-cli [--practice] --goal-file GOAL"
f=$1
shift
[ "$1" = start ] && [ "$2" = --harness ] && [ "$3" = hermes-cli ] ||
  refuse "this loader executes only \`goalcup start --harness hermes-cli\`"
shift 3
check_rest "$@"
command -v goalcup >/dev/null 2>&1 || refuse "goalcup is not on PATH"

# The only external commands, before the key is anywhere: the file's mode. BSD stat (macOS) prints
# the permission bits with `-f %Lp`; GNU stat (Linux) reads `-f` as "file system" and prints
# something else, so each form's output counts only if that form succeeded and printed octal
# digits alone. Absolute paths only, so no PATH entry or function can stand in for stat; `--`, so
# a file name starting with `-` is read as a name. Any other answer is refused, never guessed.
[ -f "$f" ] || refuse "the env file is not a regular file"
mode=
for stat in /usr/bin/stat /bin/stat; do
  [ -x "$stat" ] || continue
  mode=$("$stat" -f %Lp -- "$f" 2>/dev/null) || mode=
  case $mode in '' | *[!0-7]*) mode=$("$stat" -c %a -- "$f" 2>/dev/null) || mode= ;; esac
  case $mode in '' | *[!0-7]*) mode= ;; *) break ;; esac
done
stat=
[ -n "$mode" ] || refuse "cannot read the env file's mode"
[ "$mode" = 600 ] || refuse "env file is not mode 0600"

# KEY-IN-MEMORY: no external command below this line.
count=0
val=
while IFS= read -r line || [ -n "$line" ]; do
  case $line in
    DEEPSEEK_API_KEY=*)
      count=$((count + 1))
      val=${line#DEEPSEEK_API_KEY=}
      ;;
  esac
done <"$f"
line=
[ "$count" = 1 ] || refuse "need exactly one DEEPSEEK_API_KEY line"
case $val in \"*\") val=${val#\"}; val=${val%\"} ;; esac
case $val in
  '' | *[!A-Za-z0-9_-]*) refuse "unexpected characters in DEEPSEEK_API_KEY" ;;
esac
[ ${#val} -ge 20 ] && [ ${#val} -le 256 ] || refuse "DEEPSEEK_API_KEY has an unexpected length"
DEEPSEEK_API_KEY=$val
export DEEPSEEK_API_KEY
val=
exec goalcup start --harness hermes-cli "$@"
