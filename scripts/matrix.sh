#!/bin/sh
# scripts/matrix.sh <connector> <server-url> [options]
# scripts/matrix.sh --check-copy <config-dir> [run-id]
#
# One practice run of one connector against one upload server, start to finish, and one result
# row:
#
#     <connector> <os>-<arch> <harness_version> PASS|FAIL <date>
#
# The run is the real thing: `goalcup login`, `goalcup start --practice` (the device-flow
# approval, the clone of the matrix repository, the harness), the end of the run, the seal and
# the upload. A chat-mode connector (`interactive = true`) runs under a pseudo-terminal, gets one
# typed prompt, and is stopped by the tracer at the practice deadline. A headless connector (not
# `interactive`, like hermes-cli) runs with `--goal-file`, gets its goal from MATRIX_PROMPT in
# that file, types nothing, and ends when the harness does; the script then waits for the
# server's deadline before it asks for the server's copy. Then it checks:
#
#   1. `goalcup-verify --capture-integrity` on the local trace exits 0 (integrity valid, anchor
#      signed by the server's acknowledgement);
#   2. the server's copy: `GET /v1/runs/<id>` with the run's own credential says `sealed`, `stop`,
#      and holds exactly the bytes, records and head the local chain ends with, and the entry's
#      committed end (`final`) is that same record.
#
# The upload API has no call that returns the stored chain, so (2) is a comparison of the
# server's position with the local chain, not a download. docs/OPERATING.md, "The matrix", says
# why that is enough and what it does not prove.
#
# <connector> is `m2-fake-tui` or `m2-fake-headless` (connectors this script writes for itself
# around the test suite's fake TUI and fake headless harness, admitted with --dev; no harness, no
# login, no model) or the id of a shipped connector (`codex-cli`, `hermes-cli`, ...), which needs
# that harness installed and logged in, in a prepared configuration directory (MATRIX_CONFIG_DIR,
# below). The script reads the connector's harness.toml to tell chat mode from headless:
# GOALCUP_CONNECTORS_DIR, else the connectors directory beside the goalcup binary, else this
# checkout's; if none holds it, the connector is taken to be chat mode.
#
# Environment. The approval key is never in this script's environment, a variable, an argument,
# a log or a file it writes: you give the PATH of a file that holds it, and the script reads that
# file only at the moment it approves, straight into curl's standard input.
#   GOALCUP_TEST_APPROVAL_KEY_FILE
#                              a regular file holding the server's TEST_APPROVAL_KEY (a local
#                              `npm run dev`, or staging). The path is not secret. Unset: the
#                              approval is yours, in a browser (see below). The script takes the
#                              variable out of its environment at once, so no child inherits the
#                              path. A Docker run mounts
#                              the file read-only (-v <file>:/run/key:ro) and never uses -e.
#   GOALCUP_TEST_MEMBER_ID     the member the test approval approves as (default: member1).
#   MATRIX_REPO_URL            the empty public repository (default:
#                              https://github.com/agentcommunity/goalcup-matrix-empty).
#   MATRIX_PROMPT              what is typed into the harness, or for a headless connector
#                              written to its goal file (default: a one-line greeting).
#   MATRIX_KEY_ENV_FILE        for a connector whose harness.toml declares the credential
#                              DEEPSEEK_API_KEY (hermes-cli): a mode 0600 file with one
#                              DEEPSEEK_API_KEY=... line. The start then goes through
#                              scripts/with-deepseek-key.sh, which reads the key itself. The key
#                              never enters this script's environment, arguments, log or files,
#                              and the path is not printed or written to a file either: it is
#                              handed to the start's wrapper in that process's environment alone.
#                              Ignored (and logged so) for a connector with no such credential.
#   MATRIX_PROMPT_DELAY        seconds between "the harness is up" and the prompt (fake: 1,
#                              any other connector: 30). Chat mode only.
#   MATRIX_SUBMIT_DELAY        seconds between the typed prompt and the Enter that submits it
#                              (default 2). Text and Enter in one write are read by Codex as a
#                              paste: the text stays in the composer and nothing is submitted.
#   MATRIX_PRE_PROMPT_KEYS     keys typed once into a chat-mode harness before the prompt, as a
#                              printf format ('\033[B\r' is Down, Enter), for a first-run dialog
#                              the prompt would otherwise answer by accident. Claude Code asks
#                              "Is this a project you created or one you trust?" in every new
#                              workspace and selects "No, exit": '\033[B\r' picks "Yes, I trust
#                              this folder". No quote, % or $ in it. Default: none.
#   MATRIX_PRE_PROMPT_DELAY    seconds between "the harness is up" and those keys (default 5; the
#                              prompt follows two seconds after them, or at MATRIX_PROMPT_DELAY
#                              if that is later).
#   MATRIX_CONFIG_DIR          a prepared goalcup configuration directory for a real harness
#                              (made with `GOALCUP_CONFIG_DIR=<dir> goalcup prep`, its logins in
#                              place). Never the platform default. Left in place afterwards.
#   MATRIX_SERVER_KEY          the server's Ed25519 public key (64 hex), for goalcup-verify
#                              --dev-key. Default: the key in the run's receipt.
#   MATRIX_BIN_DIR             where goalcup, goalcup-daemon and goalcup-verify are.
#
# Options:
#   --allow-production   permit https://ingest.agentcommunity.org. Tests never use it.
#   --keep               do not delete the temporary directory (it holds the run's trace).
#   --log FILE           where the details go (default: a new file in $TMPDIR).
#   --timeout SECONDS    give up on the run after this long (default 1800).
#   --check-copy DIR [RUN]
#                        only the server-copy check, for a run you made by hand (the edge cases
#                        in docs/OPERATING.md): every entry of RUN (default: the newest) in the
#                        configuration directory DIR is verified locally and compared with the
#                        server's position for it. Starts and approves nothing; read-only.
#
# Without GOALCUP_TEST_APPROVAL_KEY_FILE the script prints the approval address and code that goalcup
# printed (nothing else) and waits while you approve, in a browser, as a member. There are two
# approvals: the login, then the run.
#
# Needs: sh, curl (7.55+), python3, script(1), mktemp; macOS or Linux (WSL2 too).
# It uses its own temporary configuration directory, work folder and HOME. It never reads or
# writes the real goalcup configuration directory, ~/.codex, ~/.claude or ~/.hermes. The one
# exception is MATRIX_CONFIG_DIR, a directory you name for a real harness, which also keeps your
# own HOME (the harness's isolated home is made from it).

set -u

PROG=$0
HERE=$(cd "$(dirname "$PROG")" && pwd) || exit 3
REPO=$(dirname "$HERE")

PRODUCTION_HOST="ingest.agentcommunity.org"
FAKE_ID="m2-fake-tui"
FAKE_HEADLESS_ID="m2-fake-headless"
DEFAULT_REPO_URL="https://github.com/agentcommunity/goalcup-matrix-empty"
LOADER="$HERE/with-deepseek-key.sh"

usage() {
  # The whole header: every comment line after the first, up to the first line that is not one.
  awk 'NR > 1 && !/^#/ { exit } NR > 1 { print }' "$PROG" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# ------------------------------------------------------------------------------ arguments

CONNECTOR=""
SERVER=""
CHECK_CFG=""
CHECK_RUN=""
ALLOW_PRODUCTION=0
KEEP=0
LOG=""
TIMEOUT=1800
while [ $# -gt 0 ]; do
  case $1 in
    -h | --help) usage ;;
    --allow-production) ALLOW_PRODUCTION=1 ;;
    --keep) KEEP=1 ;;
    --check-copy)
      [ $# -ge 2 ] || usage
      CHECK_CFG=$2
      shift
      case ${2:-} in "" | -*) ;; *)
        CHECK_RUN=$2
        shift
        ;;
      esac
      ;;
    --log)
      [ $# -ge 2 ] || usage
      LOG=$2
      shift
      ;;
    --timeout)
      [ $# -ge 2 ] || usage
      TIMEOUT=$2
      shift
      ;;
    -*) usage ;;
    *)
      if [ -z "$CONNECTOR" ]; then
        CONNECTOR=$1
      elif [ -z "$SERVER" ]; then
        SERVER=$1
      else
        usage
      fi
      ;;
  esac
  shift
done
if [ -n "$CHECK_CFG" ]; then
  [ -z "$CONNECTOR" ] || usage
  CONNECTOR=check-copy
else
  [ -n "$CONNECTOR" ] && [ -n "$SERVER" ] || usage
fi

# The approval key: only ever a FILE. A key in the environment would sit in this process's
# environment block (readable by `ps eww` and /proc/<pid>/environ) for the whole run, which no
# `unset` scrubs, so it is refused.
if [ -n "${GOALCUP_TEST_APPROVAL_KEY:-}" ]; then
  echo "matrix: GOALCUP_TEST_APPROVAL_KEY in the environment is refused (any process of yours could read it): put the key in a file and set GOALCUP_TEST_APPROVAL_KEY_FILE=<path>" >&2
  exit 2
fi
KEYFILE=${GOALCUP_TEST_APPROVAL_KEY_FILE:-}
# The path leaves the environment too, so goalcup, the daemon and the harness never inherit where
# the key file is.
unset GOALCUP_TEST_APPROVAL_KEY_FILE
if [ -n "$KEYFILE" ] && { [ ! -f "$KEYFILE" ] || [ ! -r "$KEYFILE" ]; }; then
  echo "matrix: GOALCUP_TEST_APPROVAL_KEY_FILE must name a regular file this user can read" >&2
  exit 2
fi
# The key loader's env file: a path again, and again out of the environment at once. It is named
# in no message, log or file; it reaches the start's wrapper in that one process's environment.
KEYENV=${MATRIX_KEY_ENV_FILE:-}
unset MATRIX_KEY_ENV_FILE
if [ -n "$KEYENV" ]; then
  if [ ! -f "$KEYENV" ] || [ ! -r "$KEYENV" ]; then
    echo "matrix: MATRIX_KEY_ENV_FILE must name a regular file this user can read" >&2
    exit 2
  fi
  case $KEYENV in /*) ;; *) KEYENV=$PWD/$KEYENV ;; esac
  case $KEYENV in
    *\'* | *[[:space:]]*)
      echo "matrix: the path in MATRIX_KEY_ENV_FILE may hold no space or quote" >&2
      exit 2
      ;;
  esac
fi
PRE_KEYS=${MATRIX_PRE_PROMPT_KEYS:-}
case $PRE_KEYS in
  *\'* | *%* | *\$* | *\`*)
    echo "matrix: MATRIX_PRE_PROMPT_KEYS may hold no quote, %, \$ or backquote" >&2
    exit 2
    ;;
esac
PRE_DELAY=${MATRIX_PRE_PROMPT_DELAY:-5}
SUBMIT_DELAY=${MATRIX_SUBMIT_DELAY:-2}
case $SUBMIT_DELAY in
  *[!0-9]* | "")
    echo "matrix: MATRIX_SUBMIT_DELAY is a number of seconds" >&2
    exit 2
    ;;
esac
case $PRE_DELAY in
  *[!0-9]* | "")
    echo "matrix: MATRIX_PRE_PROMPT_DELAY is a number of seconds" >&2
    exit 2
    ;;
esac
MEMBER=${GOALCUP_TEST_MEMBER_ID:-member1}
REPO_URL=${MATRIX_REPO_URL:-$DEFAULT_REPO_URL}

case $CONNECTOR in
  *[!A-Za-z0-9._-]* | "")
    echo "matrix: the connector id may hold only letters, digits, . _ -" >&2
    exit 2
    ;;
esac
case $MEMBER in
  *[!A-Za-z0-9_-]* | "")
    echo "matrix: GOALCUP_TEST_MEMBER_ID may hold only letters, digits, _ -" >&2
    exit 2
    ;;
esac
case $TIMEOUT in
  *[!0-9]* | "")
    echo "matrix: --timeout is a number of seconds" >&2
    exit 2
    ;;
esac

# The server: https://..., or http:// to this machine. Never production without the flag.
validate_server() {
  SERVER=$(printf '%s' "$SERVER" | sed 's:/*$::')
  SERVER_LC=$(printf '%s' "$SERVER" | tr 'A-Z' 'a-z')
  case $SERVER_LC in
    https://*) ;;
    http://*)
      # Plain http only to this machine: the host is exactly localhost, 127.0.0.1 or [::1], then
      # an optional :port and an optional path.
      auth=${SERVER_LC#http://}
      auth=${auth%%/*}
      case $auth in
        localhost | 127.0.0.1 | "[::1]") ;;
        localhost:* | 127.0.0.1:* | "[::1]":*)
          case ${auth##*:} in "" | *[!0-9]*) auth="" ;; esac
          ;;
        *) auth="" ;;
      esac
      [ -n "$auth" ] || {
        echo "matrix: plain http is allowed only to localhost, 127.0.0.1 or [::1] (with an optional :port)" >&2
        exit 2
      }
      ;;
    *)
      echo "matrix: the server must be https://..., or http:// to localhost" >&2
      exit 2
      ;;
  esac
  case $SERVER_LC in
    *"$PRODUCTION_HOST"*)
      if [ "$ALLOW_PRODUCTION" != 1 ]; then
        echo "matrix: refusing $PRODUCTION_HOST: tests never run against production (--allow-production overrides)" >&2
        exit 2
      fi
      ;;
  esac
}
[ -n "$CHECK_CFG" ] || validate_server

# ------------------------------------------------------------------------------ platform

case $(uname -s) in
  Darwin) OS=macos ;;
  Linux)
    OS=linux
    if grep -qi microsoft /proc/version 2>/dev/null; then OS=wsl2; fi
    ;;
  *)
    echo "matrix: macOS and Linux only" >&2
    exit 2
    ;;
esac
case $(uname -m) in
  arm64 | aarch64) ARCH=arm64 ;;
  x86_64 | amd64) ARCH=x86_64 ;;
  *) ARCH=$(uname -m) ;;
esac
PLATFORM="$OS-$ARCH"
DATE=$(date -u +%Y-%m-%d)

for tool in curl python3 script mktemp; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "matrix: $tool is needed and was not found" >&2
    exit 2
  }
done

# ------------------------------------------------------------------------------ log and state

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
[ -n "$LOG" ] || LOG="${TMPDIR:-/tmp}/goalcup-matrix-$CONNECTOR-$STAMP.log"
LOG_DIR=$(dirname "$LOG")
mkdir -p "$LOG_DIR" 2>/dev/null
: >"$LOG" || {
  echo "matrix: cannot write the log $LOG" >&2
  exit 2
}

exec 3>&2 # the terminal's stderr; after cleanup starts, fd 2 is the log
log() { printf '%s %s\n' "$(date -u +%H:%M:%SZ)" "$*" >>"$LOG"; }
say() {
  log "$*"
  printf 'matrix: %s\n' "$*" >&3
}
# A file's text, without carriage returns, under a heading.
log_file() {
  log "---- $1"
  if [ -f "$2" ]; then tr -d '\r' <"$2" | sed 's/^/    /' >>"$LOG"; fi
  log "---- end $1"
}

RESULT=FAIL
FAIL_REASON=""
HV=unreported
ROW_PRINTED=0
ROOT=""
CFG=""
OWN_CONFIG=0
STARTED_RUN=0

print_row() {
  [ "$ROW_PRINTED" = 0 ] || return 0
  ROW_PRINTED=1
  hv=$(printf '%s' "$HV" | tr -s ' \t' '__')
  [ -n "$hv" ] || hv=unreported
  if [ "$RESULT" != PASS ] && [ -n "$FAIL_REASON" ]; then
    log "FAIL: $FAIL_REASON"
    printf 'matrix: FAIL: %s\n' "$FAIL_REASON" >&3
  fi
  row="$CONNECTOR $PLATFORM $hv $RESULT $DATE"
  log "RESULT $row"
  printf '%s\n' "$row"
  printf 'log: %s\n' "$LOG"
}

cleanup() {
  trap '' INT TERM HUP
  # The shell's own notices about the jobs it ends here go to the log, not the terminal.
  exec 2>>"$LOG"
  if [ -n "$ROOT" ] && [ -d "$ROOT" ]; then {
    : >"$ROOT/done" 2>/dev/null
    # Whatever is still running from this script's own temporary directory, by that unique path.
    for pidfile in "$ROOT/login.pid" "$ROOT/start.pid"; do
      if [ -f "$pidfile" ] && [ ! -f "$ROOT/$(basename "$pidfile" .pid).rc" ]; then kill -TERM "$(cat "$pidfile")" 2>/dev/null; fi
    done
    if command -v pkill >/dev/null 2>&1; then
      pkill -TERM -f "$ROOT/" 2>/dev/null
      # The daemon this run started from its own configuration directory, and no other.
      if [ "$STARTED_RUN" = 1 ]; then pkill -TERM -f "goalcup-daemon --config ${CFG:-/nonexistent}/" 2>/dev/null; fi
    fi
    sleep 1
    if [ "$KEEP" = 1 ]; then
      log "kept $ROOT"
    else
      rm -rf "$ROOT" 2>/dev/null
      # The daemon's roots sit beside the configuration directory: ours only if ours.
      if [ "${OWN_CONFIG:-0}" = 1 ]; then rm -rf "$CFG-artifacts" "$CFG-runs" 2>/dev/null; fi
    fi
  }; fi
  print_row
}
on_signal() {
  FAIL_REASON="interrupted"
  exit 130
}
trap cleanup EXIT
trap on_signal INT TERM HUP

# ------------------------------------------------------------------------------ helpers

fail() {
  FAIL_REASON=$*
  return 1
}

shq() { # a single-quoted shell word
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

find_bin() { # name
  for dir in "${MATRIX_BIN_DIR:-}" "$REPO/target/release" "$REPO/target/debug"; do
    if [ -n "$dir" ] && [ -x "$dir/$1" ]; then
      printf '%s\n' "$dir/$1"
      return 0
    fi
  done
  command -v "$1" 2>/dev/null
}

# The server answers the common way: HTTP status to $2, body to the file $3.
http() { # method path outfile [extra curl args...]
  m=$1
  p=$2
  o=$3
  shift 3
  curl -sS --max-time 30 -X "$m" -o "$o" -w '%{http_code}' "$@" "$SERVER$p" 2>>"$LOG"
}

# The user codes goalcup printed in a log (one per line, in order).
codes_in() {
  tr -d '\r' 2>/dev/null <"$1" | sed -n 's/.*shows the code \([A-Z][A-Z][A-Z][A-Z]-[A-Z][A-Z][A-Z][A-Z]\)\..*/\1/p'
}

# Approve the first user code goalcup prints in the log $1, and no other: one code per phase (the
# login, the run). seen.txt is emptied at the start of each phase. With a key file the approval
# goes through the test route, the key piped from the file straight into curl's standard input as
# a header file, so it is in no variable, argument, environment or temp file. Without one, say what
# goalcup printed, once.
scan_and_approve() {
  [ ! -s "$ROOT/seen.txt" ] || return 0
  code=$(codes_in "$1" | head -1)
  [ -n "$code" ] || return 0
  printf '%s\n' "$code" >>"$ROOT/seen.txt"
  if [ -n "$KEYFILE" ]; then
    status=$({
      printf 'Authorization: Bearer '
      tr -d '\n' <"$KEYFILE"
      printf '\n'
    } | http POST /v1/device/approve "$ROOT/approve.out" -H @- -H 'Content-Type: application/json' \
      -d "{\"user_code\":\"$code\",\"member_id\":\"$MEMBER\",\"approve\":true}")
    log "approval of $code as $MEMBER: HTTP $status $(cat "$ROOT/approve.out" 2>/dev/null)"
    if [ "$status" != 200 ]; then
      log "the test approval was refused"
      return 1
    fi
  else
    line=$(tr -d '\r' <"$1" | grep "shows the code $code" | tail -1)
    say "no GOALCUP_TEST_APPROVAL_KEY_FILE: approve this in a browser, as a member:"
    say "$line"
  fi
  return 0
}

# A wrapper script that runs goalcup in the matrix's isolated environment. It records its own
# process id (the id goalcup keeps after the exec) so a stuck run can be ended by id.
#
# With a fifth argument "keyed", goalcup is not run directly but through scripts/with-deepseek-key.sh
# (`/bin/sh -p`), which reads the key file named by MATRIX_KEY_ENV_FILE in the wrapper's own
# environment (the caller sets it for that one process; the wrapper expands it into the loader's
# argument and drops it from the environment goalcup and the harness get). The loader finds
# `goalcup` on PATH, so the directory of the binary this script chose goes first on it. Nothing
# about the key file is written into the wrapper.
#
# GOALCUP_NO_BROWSER keeps goalcup from opening a browser page for each approval: the script
# approves through the test route or prints the address for you. (GOALCUP_TEST_NO_BROWSER is the
# same switch for a build with test-support.)
write_wrapper() { # file name "goalcup arguments" [stty] [keyed]
  {
    echo '#!/bin/sh'
    [ -z "${4:-}" ] || echo 'stty cols 120 rows 40 2>/dev/null'
    echo "echo \$\$ >$(shq "$ROOT/$2.pid")"
    echo "cd $(shq "$RUNS") || exit 1"
    printf 'exec env -u MATRIX_KEY_ENV_FILE HOME=%s GOALCUP_CONFIG_DIR=%s' "$(shq "$HOME_DIR")" "$(shq "$CFG")"
    [ -z "$CONNECTORS_ENV" ] || printf ' GOALCUP_CONNECTORS_DIR=%s' "$(shq "$CONNECTORS_ENV")"
    if [ "${5:-}" = keyed ]; then
      printf ' GOALCUP_NO_BROWSER=1 GOALCUP_TEST_NO_BROWSER=1 PATH=%s TERM=%s' \
        "$(shq "$(dirname "$GOALCUP"):$HARNESS_PATH")" "$MATRIX_TERM"
      printf ' /bin/sh -p %s "$MATRIX_KEY_ENV_FILE" %s\n' "$(shq "$LOADER")" "$3"
    else
      printf ' GOALCUP_NO_BROWSER=1 GOALCUP_TEST_NO_BROWSER=1 PATH=%s TERM=%s' "$(shq "$HARNESS_PATH")" "$MATRIX_TERM"
      printf ' %s %s\n' "$(shq "$GOALCUP")" "$3"
    fi
  } >"$1"
}

# Wait for the background job that writes its exit status to $1, scanning the output $2 for
# approvals meanwhile. 0 when it finished (status in $1), 1 on timeout, 2 on a refused approval.
wait_job() { # rcfile outfile limit
  waited=0
  while [ ! -f "$1" ]; do
    if [ "$SCAN" = 1 ]; then scan_and_approve "$2" || return 2; fi
    after_scan "$2"
    if [ "$waited" -ge "$3" ]; then return 1; fi
    sleep 2
    waited=$((waited + 2))
  done
  return 0
}
after_scan() { :; }
# Whether the log being waited on is still scanned for an approval code. It stops once the
# harness is up, so nothing the harness itself prints can ever be approved.
SCAN=1

# ------------------------------------------------------------------------------ the matrix

run_matrix() {
  GOALCUP=$(find_bin goalcup)
  VERIFY=$(find_bin goalcup-verify)
  DAEMON=$(find_bin goalcup-daemon)
  [ -n "$GOALCUP" ] && [ -n "$VERIFY" ] && [ -n "$DAEMON" ] ||
    fail "goalcup, goalcup-daemon and goalcup-verify are needed (cargo build --workspace, or MATRIX_BIN_DIR)" || return 1
  GOALCUP_VERSION=$("$GOALCUP" --version 2>&1 | head -1)
  log "matrix $CONNECTOR on $PLATFORM against $SERVER, $DATE"
  log "goalcup: $GOALCUP ($GOALCUP_VERSION); verifier: $VERIFY; daemon: $DAEMON"
  log "key file supplied: $([ -n "$KEYFILE" ] && echo yes || echo no); member: $MEMBER; repository: $REPO_URL"

  code=$(http GET /v1/health "$LOG.health")
  health=$(cat "$LOG.health" 2>/dev/null)
  rm -f "$LOG.health"
  [ "$code" = 200 ] || fail "the server did not answer /v1/health with 200 (got $code)" || return 1
  log "health: $health"

  ROOT=$(mktemp -d "${TMPDIR:-/tmp}/goalcup-matrix.XXXXXX") || fail "cannot make a temporary directory" || return 1
  made=$ROOT
  ROOT=$(cd "$made" && pwd -P) || {
    ROOT=$made
    fail "cannot enter the temporary directory $made" || return 1
  }
  [ -n "$ROOT" ] || { ROOT=$made; fail "the temporary directory has no path" || return 1; }
  case $ROOT in *\'* | *" "*) fail "the temporary directory path holds a space or quote: $ROOT" || return 1 ;; esac
  RUNS="$ROOT/runs"
  mkdir -p "$RUNS"
  : >"$ROOT/seen.txt"

  if [ -n "${MATRIX_CONFIG_DIR:-}" ]; then
    CFG=${MATRIX_CONFIG_DIR%/}
    case $CFG in
      "$HOME/Library/Application Support/goalcup" | "${XDG_CONFIG_HOME:-$HOME/.config}/goalcup" | "$HOME/.config/goalcup")
        fail "MATRIX_CONFIG_DIR is the real goalcup configuration directory; make a separate one with GOALCUP_CONFIG_DIR=<dir> goalcup prep" || return 1
        ;;
    esac
    [ -f "$CFG/prep.json" ] && [ -f "$CFG/github.json" ] || fail "MATRIX_CONFIG_DIR $CFG is not prepared (no prep.json and github.json): run GOALCUP_CONFIG_DIR=$CFG goalcup prep" || return 1
    HOME_DIR=$HOME
    OWN_CONFIG=0
  else
    CFG="$ROOT/cfg"
    HOME_DIR="$ROOT/home"
    mkdir -p "$HOME_DIR" "$ROOT/work"
    OWN_CONFIG=1
  fi
  mkdir -p "$CFG"
  chmod 700 "$CFG"

  CONNECTORS_ENV=""
  HARNESS_PATH=$PATH
  DEV_FLAG=""
  PROMPT_ENTER='\r'
  READY_MARK="Recording in chat mode"
  DELAY=${MATRIX_PROMPT_DELAY:-30}
  IS_FAKE=0
  if [ "$CONNECTOR" = "$FAKE_ID" ]; then
    IS_FAKE=1
    make_fake_connector || return 1
    DEV_FLAG="--dev"
    PROMPT_ENTER='\n'
    READY_MARK="tui: ready"
    DELAY=${MATRIX_PROMPT_DELAY:-1}
  elif [ "$CONNECTOR" = "$FAKE_HEADLESS_ID" ]; then
    IS_FAKE=1
    make_fake_headless_connector || return 1
    DEV_FLAG="--dev"
  fi
  MATRIX_TERM=${MATRIX_TERM:-$([ "$CONNECTOR" = "$FAKE_ID" ] && echo dumb || echo xterm-256color)}
  read_manifest || return 1
  PROMPT=${MATRIX_PROMPT:-"Goal Cup matrix check: reply with the single word ok."}

  if [ "$OWN_CONFIG" = 1 ]; then
    # What `goalcup prep` would have settled. A practice clone is anonymous and installs no push
    # helper, so the token is format-valid and fake: it is never sent anywhere.
    printf '{\n  "harness": "%s",\n  "repo_url": "%s",\n  "work_root": "%s"\n}\n' \
      "$CONNECTOR" "$REPO_URL" "$ROOT/work" >"$CFG/prep.json"
    printf '{\n  "repo_url": "%s",\n  "token": "ghp_matrixfaketoken00000000000000000000"\n}\n' \
      "$REPO_URL" >"$CFG/github.json"
    chmod 600 "$CFG/prep.json" "$CFG/github.json"
    if [ "$IS_FAKE" = 0 ]; then
      log "note: a real harness without MATRIX_CONFIG_DIR has no login in a fresh configuration"
    fi
  fi

  # ---- login
  STARTED_RUN=1
  say "login against $SERVER"
  write_wrapper "$ROOT/login-wrapper.sh" login "login --server $(shq "$SERVER")"
  # (Started from a subshell that exits at once, so it is not a job of this shell: ending it
  # later prints no job notice. Its status is the file the loop waits for.)
  ( ( (sh "$ROOT/login-wrapper.sh" >"$ROOT/login.out" 2>&1); echo $? >"$ROOT/login.rc") 2>/dev/null & )
  wait_job "$ROOT/login.rc" "$ROOT/login.out" 960
  w=$?
  log_file "goalcup login" "$ROOT/login.out"
  case $w in
    1) fail "the login was not approved in time" || return 1 ;;
    2) fail "the test approval was refused (wrong key, member, or not a test server)" || return 1 ;;
  esac
  [ "$(cat "$ROOT/login.rc")" = 0 ] ||
    fail "goalcup login exited $(cat "$ROOT/login.rc"): $(last_line "$ROOT/login.out")" || return 1

  # ---- start --practice
  : >"$ROOT/seen.txt"
  WRAP="$ROOT/start-wrapper.sh"
  START_ARGS="start --harness $CONNECTOR $DEV_FLAG --practice"
  WRAP_KIND=""
  KEYED_START=""
  if [ "$MODE" = headless ]; then
    # The goal is a file: the goal of a headless harness is its only input, typed by nobody.
    printf '%s\n' "$PROMPT" >"$ROOT/goal.txt"
    START_ARGS="$START_ARGS --goal-file $(shq "$ROOT/goal.txt")"
  else
    WRAP_KIND=stty
  fi
  if [ -n "$KEYENV" ] && [ "$CRED_ENV" = DEEPSEEK_API_KEY ]; then
    [ -f "$LOADER" ] || fail "the key loader is missing: $LOADER" || return 1
    KEYED_START=keyed
    log "the start goes through scripts/with-deepseek-key.sh (key file supplied; its path is not logged)"
  elif [ -n "$KEYENV" ]; then
    log "MATRIX_KEY_ENV_FILE is ignored: $CONNECTOR declares no DEEPSEEK_API_KEY credential"
  elif [ "$CRED_ENV" = DEEPSEEK_API_KEY ]; then
    log "note: $CONNECTOR declares the credential DEEPSEEK_API_KEY and MATRIX_KEY_ENV_FILE is not set"
  fi
  write_wrapper "$WRAP" start "$START_ARGS" "$WRAP_KIND" "$KEYED_START"
  if [ "$MODE" = headless ]; then
    say "start --practice, headless (the harness ends the run; then waiting for the server's deadline)"
    # No terminal and nothing typed: the harness's input is empty. (Started from a subshell, as the
    # login is.) The key file's path is in the wrapper process's environment alone.
    ( ( (MATRIX_KEY_ENV_FILE=$KEYENV sh "$WRAP" >"$ROOT/start.out" 2>&1 </dev/null); echo $? >"$ROOT/start.rc") 2>/dev/null & )
  else
    # The feeder holds the terminal's input open until the run is over (an end of input would be
    # an end of the chat) and types the prompt once, when the main loop says the harness is up.
    # The status file is written by the terminal's side of the pipe, so it appears when script
    # ends, not when the feeder does.
    FEED="while [ ! -f $(shq "$ROOT/done") ]; do
    if [ -f $(shq "$ROOT/prekeys-send") ] && [ ! -f $(shq "$ROOT/prekeys-sent") ]; then
      printf '$PRE_KEYS'; : > $(shq "$ROOT/prekeys-sent")
    fi
    if [ -f $(shq "$ROOT/send") ] && [ ! -f $(shq "$ROOT/sent") ]; then
      printf '%s' $(shq "$PROMPT"); sleep $SUBMIT_DELAY; printf '$PROMPT_ENTER'; : > $(shq "$ROOT/sent")
    fi
    sleep 1
  done"
    say "start --practice (the run lasts the server's practice length; waiting for its deadline)"
    if [ "$OS" = macos ]; then
      ( ( (eval "$FEED") | (MATRIX_KEY_ENV_FILE=$KEYENV script -q /dev/null sh "$WRAP" >"$ROOT/start.out" 2>&1; echo $? >"$ROOT/start.rc") 2>/dev/null ) & )
    else
      ( ( (eval "$FEED") | (MATRIX_KEY_ENV_FILE=$KEYENV script -qec "sh $(shq "$WRAP")" /dev/null >"$ROOT/start.out" 2>&1; echo $? >"$ROOT/start.rc") 2>/dev/null ) & )
    fi
    READY_AT=""
    PRE_AT=""
    after_scan() {
      if [ -n "$READY_AT" ]; then
        now=$(date +%s)
        if [ -n "$PRE_KEYS" ] && [ -z "$PRE_AT" ] && [ $((now - READY_AT)) -ge "$PRE_DELAY" ]; then
          : >"$ROOT/prekeys-send"
          PRE_AT=$now
          log "the first-run keys are typed"
        fi
        if [ ! -f "$ROOT/send" ] && [ $((now - READY_AT)) -ge "$DELAY" ]; then
          if [ -z "$PRE_KEYS" ] || { [ -f "$ROOT/prekeys-sent" ] && [ $((now - PRE_AT)) -ge 2 ]; }; then
            : >"$ROOT/send"
          fi
        fi
      fi
      if [ -z "$READY_AT" ] && grep -q "$READY_MARK" "$ROOT/start.out" 2>/dev/null; then
        READY_AT=$(date +%s)
        SCAN=0
        log "the harness is up"
      fi
    }
  fi
  wait_job "$ROOT/start.rc" "$ROOT/start.out" "$TIMEOUT"
  w=$?
  : >"$ROOT/done"
  log_file "goalcup start ($([ "$MODE" = headless ] && echo output || echo terminal))" "$ROOT/start.out"
  case $w in
    1) fail "the run did not finish within ${TIMEOUT}s" || return 1 ;;
    2) fail "the test approval was refused (wrong key, member, or not a test server)" || return 1 ;;
  esac
  START_RC=$(cat "$ROOT/start.rc")
  log "goalcup start exit status: $START_RC$([ "$MODE" = headless ] || echo " (script's status; the harness is stopped by the deadline, so a non-zero status is not itself a failure)")"

  tr -d '\r' <"$ROOT/start.out" >"$ROOT/start.txt"

  # ---- what the run left (the newest run record this start wrote)
  RUN_JSON=""
  ls -t "$CFG"/runs/*/run.json 2>/dev/null >"$ROOT/runs.txt"
  while IFS= read -r f; do
    # Not older than the wrapper (whole seconds on some systems, so equal counts as new).
    if ! [ "$WRAP" -nt "$f" ]; then
      RUN_JSON=$f
      break
    fi
  done <"$ROOT/runs.txt"
  [ -n "$RUN_JSON" ] ||
    fail "no run record under $CFG/runs; the tracer's last words: $(last_line "$ROOT/start.txt")" || return 1
  RUN_DIR=$(dirname "$RUN_JSON")
  RUN_ID=$(py_get "$RUN_JSON" run_id)
  TRACE=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["entries"][-1]["trace_dir"])' "$RUN_JSON")
  ENTRY_ID=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["entries"][-1]["entry_id"])' "$RUN_JSON")
  log "run $RUN_ID, entry $ENTRY_ID, trace $TRACE"
  if [ -f "$TRACE/chain.jsonl" ]; then
    # Read before the end-of-run checks, so a FAIL row still names the harness version.
    HV=$(python3 - "$TRACE/chain.jsonl" <<'PY'
import json, sys
def find(o):
    if isinstance(o, dict):
        if isinstance(o.get("harness_version"), str):
            return o["harness_version"]
        for v in o.values():
            r = find(v)
            if r:
                return r
    elif isinstance(o, list):
        for v in o:
            r = find(v)
            if r:
                return r
    return None
for line in open(sys.argv[1]):
    r = find(json.loads(line))
    if r:
        print(r.splitlines()[0][:64])
        break
PY
    )
    [ -n "$HV" ] || HV=unreported
    log "harness version: $HV"
  fi

  # The tracer's own words about how the run ended. A chat-mode run's `deadline:` line goes to the
  # run's host.log (the terminal belongs to the harness), a headless run's to the terminal, and
  # `harness exited:` and `uploaded:` to the terminal in both: so both files are read.
  { cat "$ROOT/start.txt"; [ ! -f "$RUN_DIR/host.log" ] || tr -d '\r' <"$RUN_DIR/host.log"; } >"$ROOT/ended.txt"

  COPY_TRIES=6
  COPY_SLEEP=5
  if [ "$MODE" = headless ]; then
    # The harness ends the run when it is done. It ended at the deadline (a long goal) or on its
    # own with exit code 0; any other exit is a harness that did not do its job.
    if ! grep -q '^deadline:' "$ROOT/ended.txt"; then
      exited=$(grep '^harness exited:' "$ROOT/ended.txt" | tail -1 | cut -c1-200)
      [ -n "$exited" ] || exited="no harness-exited line; the tracer's last words: $(last_line "$ROOT/start.txt")"
      [ "$exited" = "harness exited: exit code 0." ] ||
        fail "the headless harness did not finish its goal: $exited" || return 1
      # Over on the harness's side, open on the server's until its deadline (the practice length).
      COPY_SLEEP=10
      COPY_TRIES=$((TIMEOUT / COPY_SLEEP + 1))
      say "the harness finished its goal; the server's copy is checked after the server's deadline"
    fi
  else
    grep -q '^deadline:' "$ROOT/ended.txt" ||
      fail "the run did not end at its deadline (no 'deadline:' line in the terminal output or host.log); the tracer's last words: $(last_line "$ROOT/ended.txt")" || return 1
  fi
  grep -q '^uploaded:.*the seal' "$ROOT/ended.txt" ||
    fail "the tracer did not report the upload finished with the seal (no 'uploaded: ... and the seal' line)" || return 1
  if [ "$CONNECTOR" = "$FAKE_ID" ]; then
    grep -q 'tui: noted' "$ROOT/start.txt" || fail "the typed prompt never reached the harness" || return 1
  fi
  [ -f "$TRACE/chain.jsonl" ] || fail "no chain.jsonl in the trace" || return 1

  # ---- 1. the local verifier, 2. the server's copy
  verify_trace "$TRACE" || return 1
  compare_copy "$RUN_ID" "$ENTRY_ID" "$TRACE" "$RUN_DIR/credential" 1 || return 1

  RESULT=PASS
  return 0
}

# The local verifier on one trace: exit 0, integrity valid, and the anchor signed by the server's
# acknowledgement. A server whose signing key this build does not pin (local, staging) is
# verified under a key supplied for the run (--dev-key; MATRIX_SERVER_KEY, else the key in the
# run's own receipt), and the log says so.
verify_trace() { # trace [allow_incomplete]
  "$VERIFY" --capture-integrity "$1" >"$ROOT/verify.out" 2>"$ROOT/verify.err"
  VRC=$?
  KEYNOTE="the pinned server key"
  VERIFY_NOTE=""
  if [ "$VRC" = 2 ] && grep -q receipt_key_not_accepted "$ROOT/verify.out"; then
    SKEY=${MATRIX_SERVER_KEY:-$(py_get "$1/receipt.json" public_key)}
    log "verify without a supplied key: exit $VRC; retrying with --dev-key (the server's own signing key is not pinned in this build for $SERVER)"
    "$VERIFY" --capture-integrity "$1" --dev-key "$SKEY" >"$ROOT/verify.out" 2>"$ROOT/verify.err"
    VRC=$?
    KEYNOTE="a key supplied for this run (key_source: supplied)"
  fi
  log_file "goalcup-verify --capture-integrity (stderr)" "$ROOT/verify.err"
  log "goalcup-verify --capture-integrity $1: exit $VRC, verified under $KEYNOTE"
  # Exit 1 is incomplete coverage: a failure for a run that ended cleanly, but the expected
  # state of a segment the tracker had to close from the tail of its chain (a kill, a reboot).
  # The caller who knows that (--check-copy) allows it; the server's copy must still match.
  if [ "$VRC" = 1 ] && [ "${2:-0}" = 1 ]; then
    log "goalcup-verify exit 1: incomplete coverage, allowed for this check"
    VERIFY_NOTE=" (verifier exit 1: incomplete coverage)"
    return 0
  fi
  [ "$VRC" = 0 ] || fail "goalcup-verify --capture-integrity exited $VRC" || return 1
  [ "$(py_get "$ROOT/verify.out" integrity)" = valid ] || fail "the verifier's report says integrity is not valid" || return 1
  [ "$(py_get "$ROOT/verify.out" anchor)" = signed ] ||
    fail "the verifier's anchor is $(py_get "$ROOT/verify.out" anchor), not signed: the server's acknowledgement was not verified" || return 1
}

# The server's copy of one entry against the local chain ($ROOT/verify.out is that trace's report).
# The upload API returns a position, not the stored bytes, so this compares what the server says it
# holds (offset, record count, head, committed end, last acknowledgement) with the local chain
# the verifier has just checked end to end. With strict=1 (the last entry of a finished run) the
# run must also be sealed and over.
compare_copy() { # run_id entry_id trace credential_file strict
  tries=0
  while :; do
    # The credential goes to curl as a header on its stdin; it is in no argument, log or file.
    status=$(python3 -c 'import json,sys; print("Authorization: Bearer " + json.load(open(sys.argv[1]))["credential"])' "$4" |
      http GET "/v1/runs/$1?entry_id=$2" "$ROOT/server.json" -H @-)
    log "GET /v1/runs/$1?entry_id=$2: HTTP $status $(cat "$ROOT/server.json" 2>/dev/null)"
    [ "$status" = 200 ] || fail "GET /v1/runs/$1 with the run's credential answered HTTP $status" || return 1
    [ "$5" = 1 ] || break
    [ "$(py_get "$ROOT/server.json" status)" = sealed ] && break
    tries=$((tries + 1))
    [ "$tries" -lt "${COPY_TRIES:-6}" ] || break
    sleep "${COPY_SLEEP:-5}"
  done
  python3 - "$ROOT/server.json" "$ROOT/verify.out" "$3/chain.jsonl" "$5" >"$ROOT/compare.out" 2>&1 <<'PY'
import json, os, sys
server = json.load(open(sys.argv[1]))
verify = json.load(open(sys.argv[2]))
chain_bytes = os.path.getsize(sys.argv[3])
strict = sys.argv[4] == "1"
records = verify["validated_records"]
head = verify["validated_head"]
final = server.get("final") or {}
checks = [
    ("server offset = local chain bytes", server.get("offset"), chain_bytes),
    ("verifier validated all local bytes", verify["validated_bytes"], chain_bytes),
    ("server next_seq = local record count", server.get("next_seq"), records),
    ("server head = local chain head", server.get("head"), head),
    ("last acknowledged hash = local chain head", (server.get("last_ack") or {}).get("hash"), head),
]
if strict or verify.get("sealed"):
    checks += [
        ("committed end (final).seq = last record", final.get("seq"), records - 1),
        ("committed end (final).head = local chain head", final.get("head"), head),
    ]
if strict:
    checks += [
        ("status is sealed", server.get("status"), "sealed"),
        ("the run is over (stop)", server.get("stop"), True),
    ]
ok = True
for name, got, want in checks:
    good = got == want
    ok = ok and good
    print(("ok    " if good else "FAIL  ") + f"{name}: server={got!r} local={want!r}")
sys.exit(0 if ok else 1)
PY
  CRC=$?
  log_file "server copy of entry $2 against the local chain" "$ROOT/compare.out"
  [ "$CRC" = 0 ] || fail "the server's copy of entry $2 does not match the local chain (see the comparison in the log)" || return 1
}

# --check-copy: the server-copy check alone, for the runs the edge cases leave. Every entry of
# the run in the configuration directory: the local trace is verified, and the server's position
# for that entry (GET with the run's own credential) is compared with it. Nothing is started,
# approved or changed.
run_check() {
  VERIFY=$(find_bin goalcup-verify)
  [ -n "$VERIFY" ] || fail "goalcup-verify is needed (cargo build --workspace, or MATRIX_BIN_DIR)" || return 1
  CFG=${CHECK_CFG%/}
  if [ -n "$CHECK_RUN" ]; then
    RUN_JSON="$CFG/runs/$CHECK_RUN/run.json"
  else
    RUN_JSON=$(ls -t "$CFG"/runs/*/run.json 2>/dev/null | head -1)
  fi
  [ -n "$RUN_JSON" ] && [ -f "$RUN_JSON" ] || fail "no run record under $CFG/runs" || return 1
  RUN_DIR=$(dirname "$RUN_JSON")
  RUN_ID=$(py_get "$RUN_JSON" run_id)
  HV=$RUN_ID
  SERVER=$(py_get "$RUN_JSON" server_url)
  [ -n "$SERVER" ] || fail "run $RUN_ID has no server (a local-only run has no server copy)" || return 1
  validate_server
  ROOT=$(mktemp -d "${TMPDIR:-/tmp}/goalcup-matrix.XXXXXX") || fail "cannot make a temporary directory" || return 1
  log "check-copy of run $RUN_ID on $SERVER, configuration $CFG"
  ok=1
  n=0
  # One "entry_id=trace_dir" per line, read a line at a time so a path may hold spaces. (fd 4, so
  # nothing run inside the loop can read the list from standard input.)
  python3 -c 'import json,sys
for e in json.load(open(sys.argv[1]))["entries"]:
    print(e["entry_id"] + "=" + e["trace_dir"])' "$RUN_JSON" >"$ROOT/entries.txt"
  while IFS= read -r pair <&4; do
    entry=${pair%%=*}
    trace=${pair#*=}
    say "entry $entry (segment $n)"
    if [ ! -f "$trace/chain.jsonl" ]; then
      FAIL_REASON="entry $entry has no local chain at $trace"
      ok=0
    elif ! verify_trace "$trace" 1 || ! compare_copy "$RUN_ID" "$entry" "$trace" "$RUN_DIR/credential" 0; then
      say "entry $entry: $FAIL_REASON"
      ok=0
    else
      say "entry $entry: verifier exit $VRC${VERIFY_NOTE:+ (incomplete coverage)}, server copy matches"
    fi
    n=$((n + 1))
  done 4<"$ROOT/entries.txt"
  [ "$n" -gt 0 ] || fail "run $RUN_ID has no entries" || return 1
  [ "$ok" = 1 ] || return 1
  RESULT=PASS
}

# The last non-empty line of a file, carriage returns removed, cut to 300 characters.
last_line() { # file
  tr -d '\r' 2>/dev/null <"$1" | grep -v '^[[:space:]]*$' | tail -1 | cut -c1-300
}

# The value of a top-level key of a JSON file ("" when absent).
py_get() { # file key
  python3 -c 'import json,sys
try:
    v = json.load(open(sys.argv[1])).get(sys.argv[2], "")
except Exception:
    v = ""
print(v if not isinstance(v, bool) else str(v).lower())' "$1" "$2"
}

# What the connector's harness.toml says about how it runs: MODE (chat when `interactive = true`,
# headless otherwise: it takes its goal as an argument) and CRED_ENV (the `capture.credential`
# variable, or empty). Read the way goalcup finds its connectors: GOALCUP_CONNECTORS_DIR, else the
# directory beside the binary, else this checkout's. A manifest nowhere is taken to be chat mode,
# the only mode this script drove before it read manifests. One key per line, as the shipped
# manifests have them.
read_manifest() {
  MODE=chat
  CRED_ENV=""
  manifest=""
  if [ -n "$CONNECTORS_ENV" ]; then
    manifest="$CONNECTORS_ENV/$CONNECTOR/harness.toml"
  else
    bindir=$(dirname "$GOALCUP")
    for dir in "${GOALCUP_CONNECTORS_DIR:-}" "$bindir/../share/goalcup/connectors" "$bindir/../../connectors" "$REPO/connectors"; do
      if [ -n "$dir" ] && [ -f "$dir/$CONNECTOR/harness.toml" ]; then
        manifest="$dir/$CONNECTOR/harness.toml"
        break
      fi
    done
  fi
  if [ ! -f "$manifest" ]; then
    log "note: no harness.toml found for $CONNECTOR; taking it to be chat mode (set GOALCUP_CONNECTORS_DIR)"
    return 0
  fi
  cat >"$ROOT/manifest.py" <<'PY'
import re, sys
table = ""
interactive = False
credential = ""
for raw in open(sys.argv[1]):
    line = raw.strip()
    if line.startswith("#") or not line:
        continue
    m = re.match(r"^\[+([^\]]+)\]+\s*(#.*)?$", line)
    if m:
        table = m.group(1).strip()
        continue
    m = re.match(r"^([A-Za-z0-9_]+)\s*=\s*(.*?)\s*(#.*)?$", line)
    if not m:
        continue
    key, value = m.group(1), m.group(2)
    if table == "capture" and key == "interactive":
        interactive = value == "true"
    elif table == "capture.credential" and key == "env":
        credential = value.strip(chr(34))
print("mode=" + ("chat" if interactive else "headless"))
print("credential=" + credential)
PY
  facts=$(python3 "$ROOT/manifest.py" "$manifest") || fail "cannot read $manifest" || return 1
  MODE=$(printf '%s\n' "$facts" | sed -n 's/^mode=//p')
  CRED_ENV=$(printf '%s\n' "$facts" | sed -n 's/^credential=//p')
  log "connector $CONNECTOR: $MODE mode${CRED_ENV:+, credential $CRED_ENV}"
}

# The connector for m2-fake-tui: this script's own copy of the test suite's chat-mode fake, in
# a connectors directory of its own (so it is the only connector, and only --dev admits it).
make_fake_connector() {
  FIXTURE="$REPO/crates/goalcup-cli/tests/fixtures/m2-fake-tui.sh"
  [ -f "$FIXTURE" ] || fail "the fake TUI fixture is missing: $FIXTURE" || return 1
  CONNECTORS_ENV="$ROOT/connectors"
  mkdir -p "$CONNECTORS_ENV/$FAKE_ID" "$ROOT/bin" "$ROOT/fake-out"
  {
    echo '#!/usr/bin/env node'
    printf 'M2_FAKE_OUT=%s\nexport M2_FAKE_OUT\n' "$(shq "$ROOT/fake-out")"
    echo 'if [ "$1" = "--version" ]; then echo "matrix-fake-tui 1.0.0"; exit 0; fi'
    sed '1d' "$FIXTURE"
  } >"$ROOT/bin/$FAKE_ID"
  # `#!/usr/bin/env node` as an npm harness has it: the `node` found is a stand-in on the PATH
  # the tracker gives the harness, which is the participant's own.
  printf '#!/bin/sh\nexec /bin/sh "$@"\n' >"$ROOT/bin/node"
  chmod 755 "$ROOT/bin/$FAKE_ID" "$ROOT/bin/node"
  HARNESS_PATH="$ROOT/bin:$PATH"
  cat >"$CONNECTORS_ENV/$FAKE_ID/harness.toml" <<EOF
schema_version = 1
id = "$FAKE_ID"

[capture]
interactive = true
auth_hosts = ["auth.openai.com"]
continuation = "chat: the participant types the prompt and may type keep going"
args = ["chat", "-C", "{workspace}"]
resume_args = ["resume", "--last"]

[capture.executable]
program = "$FAKE_ID"
version_args = ["--version"]

[capture.env]
SHELL = "/bin/zsh"

[capture.home]
run_home = "empty"
config_home = "shared"
env = "CODEX_HOME"
real_dirs = ["{user_home}/.codex"]
skills_dir = "skills"

[capture.ca]
env = ["CODEX_CA_CERTIFICATE"]
mode = "extend"

[capture.identity]
source = "none"

[[capture.native]]
name = "session"
root = "{config_home}/sessions"
pattern = "*.jsonl"
form = "append_only"
since_run_start = true

[capture.termination]
strategy = "process_group"
grace_seconds = 2

[[capture.hosts]]
host = "localhost"
evidence_paths = ["/v1/responses"]
protocol_hint = "openai-responses"
EOF
}

# The connector for m2-fake-headless: this script's own copy of the test suite's fake headless
# harness (tests/fixtures/m2-fake-harness.sh), as a headless connector: the goal after `--`, stdin
# empty. The `stop` file is there from the start, so the fake ends at once with exit code 0, as a
# one-turn harness does (hermes-cli): the run ends on the harness's side long before the server's
# deadline.
make_fake_headless_connector() {
  FIXTURE="$REPO/crates/goalcup-cli/tests/fixtures/m2-fake-harness.sh"
  [ -f "$FIXTURE" ] || fail "the fake headless harness fixture is missing: $FIXTURE" || return 1
  CONNECTORS_ENV="$ROOT/connectors"
  mkdir -p "$CONNECTORS_ENV/$FAKE_HEADLESS_ID" "$ROOT/bin" "$ROOT/fake-out"
  : >"$ROOT/fake-out/stop"
  {
    echo '#!/bin/sh'
    printf 'M2_FAKE_OUT=%s\nexport M2_FAKE_OUT\n' "$(shq "$ROOT/fake-out")"
    sed '1d' "$FIXTURE"
  } >"$ROOT/bin/$FAKE_HEADLESS_ID"
  chmod 755 "$ROOT/bin/$FAKE_HEADLESS_ID"
  HARNESS_PATH="$ROOT/bin:$PATH"
  cat >"$CONNECTORS_ENV/$FAKE_HEADLESS_ID/harness.toml" <<EOF
schema_version = 1
id = "$FAKE_HEADLESS_ID"

[capture]
auth_hosts = ["auth.openai.com"]
continuation = "none: the fake harness takes no input after its goal"
args = ["run", "-C", "{workspace}", "--"]

[capture.executable]
program = "$FAKE_HEADLESS_ID"

[capture.home]
run_home = "empty"
config_home = "shared"
env = "CODEX_HOME"
real_dirs = ["{user_home}/.codex"]

[capture.ca]
env = ["CODEX_CA_CERTIFICATE"]
mode = "extend"

[capture.identity]
source = "none"

[[capture.native]]
name = "session"
root = "{config_home}/sessions"
pattern = "*.jsonl"
form = "append_only"
since_run_start = true

[capture.termination]
strategy = "process_group"
grace_seconds = 2

[[capture.hosts]]
host = "localhost"
evidence_paths = ["/v1/responses"]
protocol_hint = "openai-responses"
EOF
}

if [ -n "$CHECK_CFG" ]; then run_check; else run_matrix; fi
if [ "$RESULT" != PASS ] && [ -n "${RUN_DIR:-}" ] && [ -f "$RUN_DIR/host.log" ]; then
  log_file "host.log (the tracer's messages during the run)" "$RUN_DIR/host.log"
fi
[ "$RESULT" = PASS ]
exit $?
