#!/bin/sh
# scripts/matrix.sh <connector> <server-url> [options]
# scripts/matrix.sh m2-fake-tui <server-url> --refusals
# scripts/matrix.sh --check-copy <config-dir> [run-id]
#
# One practice run (or, with --test, one organizer test run sealed after --duration) of one
# connector against one upload server, start to finish, and one result row:
#
#     <connector> <os>-<arch> <harness_version> PASS|FAIL|NOT-VERIFIED <date>
#
# Three kinds of cell ("The project folder", below):
#   practice   (the default) an empty project folder and no repository, as a participant's
#              practice run.
#   --test     a test run in a project folder holding the starting kit (a .gitignore and a
#              .env.local with GH_TOKEN from MATRIX_GH_TOKEN_FILE, or from `gh auth token` only
#              with MATRIX_ALLOW_LOGIN_TOKEN=1), with the repository MATRIX_REPO names (or, for the
#              login token, one the kit makes with `gh repo create`), passed as
#              `goalcup start --test --repo`, as a player's.
#   --refusals the start's refusals before the approval, with no harness and no token.
#
# The run is the real thing: `goalcup login`, `goalcup start --practice` (the device-flow
# approval, the harness in the project folder), the end of the run, the seal and the upload.
# Every connector is a chat-mode one (`interactive = true`, D57): it runs under a
# pseudo-terminal, gets one typed prompt, and is stopped by goalcup at the practice deadline.
# Then it checks:
#
#   1. `goalcup-verify --capture-integrity` on the local trace exits 0 (integrity valid, anchor
#      signed by the server's acknowledgement);
#   2. the server's copy: `GET /v1/runs/<id>` with the run's own credential holds exactly the bytes,
#      records and head the local chain ends with, and the entry's committed end (`final`) is that
#      same record; a practice run's also says `sealed` and `stop` (a test run's does not: F2);
#   3. for a real harness, the agent worked: `goalcup-verify --research-json` on the trace shows
#      at least one model exchange that carries output (a usage observation with output or
#      completion tokens above 0, or a decoded assistant message). A prompt swallowed by a
#      first-run dialog or split into keys leaves none, and the row FAILs. And the workspace
#      changed: the run record's `workspace` (the project folder) holds a path beyond the starting
#      kit (the .git folder aside), or a commit (on any ref, pushed or not) was made since the
#      run started. A run that only named its session, or stalled at a permission prompt,
#      changes nothing and FAILs. MATRIX_EXPECT_FILES=0 skips this for a prompt that writes nothing.
#
# The upload API has no call that returns the stored chain, so (2) is a comparison of the
# server's position with the local chain, not a download. docs/OPERATING.md, "The matrix", says
# why that is enough and what it does not prove.
#
# A long run (about 30 minutes or more) prunes its acknowledged chain parts on this machine, so its
# trace holds `chain.<offset>.jsonl` parts and no chain.jsonl, and the local verifier says
# chain_pruned_locally. Then the checks run on the server's copy, rebuilt by scripts/fetch-run.sh
# (read only), plus one more: every local part is byte for byte the copy's bytes at its offset, and
# the last one ends where the copy ends. The fetch needs MATRIX_FETCH_INDEX or DATABASE_URL, and
# MATRIX_FETCH_STORE or CLOUDFLARE_ACCOUNT_ID with CLOUDFLARE_API_TOKEN (below). Without them the
# row says NOT-VERIFIED ("NOT VERIFIED: long run, server copy not fetched"), exit 4: not a PASS,
# and not a FAIL of the run.
#
# <connector> is `m2-fake-tui` (a connector this script writes for itself around the test
# suite's fake TUI, admitted with --dev; no harness, no login, no model) or the id of a shipped
# connector (`codex-cli`, `hermes-cli`, ...), which needs that harness installed and logged in,
# in a prepared configuration directory (MATRIX_CONFIG_DIR, below). The connector's harness.toml
# must be found: GOALCUP_CONNECTORS_DIR, else the connectors directory beside the goalcup binary,
# else this checkout's.
#
# Environment. The approval key is never in this script's environment, a variable, an argument,
# a log or a file it writes: you give the PATH of a file that holds it, and the script reads that
# file only at the moment it approves, straight into curl's standard input.
#   GOALCUP_TEST_APPROVAL_KEY_FILE
#                              a regular file holding the server's TEST_APPROVAL_KEY (a local
#                              `npm run dev`, or staging). The path is not secret. Unset: the
#                              approval is yours, in a browser (see below). The script takes the
#                              variable out of its environment at once, so no child inherits the
#                              path. A Docker cell (linux-matrix.sh) has no key at all (below).
#   MATRIX_CODES_FILE          a file to append each device code to, as `login <code>` and then
#                              `start <code>`, instead of approving it (no key is given): the host
#                              approves them (linux-matrix.sh, for a Docker cell, where the key
#                              must never be in the container the agent runs in).
#   MATRIX_ALLOW_SERVER        the one https:// server, named exactly, to use although it is not
#                              staging or this machine. The approval key is sent to the server, so
#                              only the staging Worker
#                              (https://goalcup-ingest-staging.b-9a5.workers.dev) and
#                              http://localhost, http://127.0.0.1 or http://[::1] (optional :port)
#                              are accepted; anything else is refused without this. It never lifts
#                              the production refusal. Removed from the environment at once.
#   GOALCUP_TEST_MEMBER_ID     the member the test approval approves as (default: member1).
#   MATRIX_REPO                a test run's repository, <owner>/<name>, instead of the one the kit
#                              makes: new, empty and public. Never the organisers' practice
#                              repository (agentcommunity/goalcup-matrix-empty). A practice run has
#                              none and ignores it. The agent pushes to it with the token in the
#                              project's .env.local, so that login must be able to.
#   MATRIX_ALLOW_LOGIN_TOKEN   1: a --test cell may give the agent `gh auth token`, the WHOLE gh login
#                              (every scope, every repository of the account), when no
#                              MATRIX_GH_TOKEN_FILE is given. Without either, a --test cell refuses
#                              to start (exit 2). One loud line says the login token is being given.
#                              Removed from the environment at once.
#   MATRIX_GH_TOKEN_FILE       a --test cell's GitHub token for the agent, from this file: the way to
#                              run a test cell. A fine-grained token whose resource owner is a
#                              throwaway account or organisation, Contents read and write only, with
#                              the cell's repository under that owner, named in MATRIX_REPO (required
#                              with this variable: the kit makes no repository for it). A regular
#                              file, mode 0600, outside the checkout,
#                              holding one line, `GH_TOKEN=<value>` or just the value (visible
#                              ASCII, no quotes); a gh login (gho_) or classic (ghp_) token is
#                              refused. Never echoed, never in an argument. The log says
#                              which source was used. The agent can read .env.local, and a trace
#                              could hold the token; `gh auth token` is the whole login, which
#                              reaches all your repositories. Removed from the environment at once. A Docker cell ignores it (the
#                              container's own gh login is used).
#   MATRIX_CELL                the cell's name in the repository the kit makes,
#                              goalcup-kit-<cell>-<6 hex digits> (default: the connector id).
#   MATRIX_PROMPT              what is typed into the harness (default: one line asking for a
#                              file matrix-ok.txt holding "ok").
#   MATRIX_PROMPT_FILE         a regular file holding the prompt instead (UTF-8; long workloads).
#                              Not together with MATRIX_PROMPT. The harness gets
#                              it typed on ONE line: every run of white space, newlines included,
#                              becomes one space, because a newline typed into a TUI submits (or
#                              splits) the prompt early. Control characters other than tab and
#                              newline are refused. A long prompt reaches the TUI as a paste; give
#                              it time with MATRIX_SUBMIT_DELAY before the Enter. A harness that
#                              has switched bracketed paste on (ESC[?2004h before the prompt) gets
#                              the prompt wrapped in ESC[200~ ... ESC[201~.
#   MATRIX_PROMPT_DELAY        seconds between "the harness is up" and the prompt (fake: 1,
#                              any other connector: 30).
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
#   MATRIX_EXPECT_FILES        1 (default): a real harness's run must change its workspace (check
#                              3); 0: a custom prompt that writes nothing, the model check only.
#   MATRIX_CONFIG_DIR          a prepared goalcup configuration directory for a real harness
#                              (made with `GOALCUP_CONFIG_DIR=<dir> goalcup setup`, the harness
#                              signed in). Never the platform default. Left in place afterwards.
#   MATRIX_SERVER_KEY          the server's Ed25519 public key (64 hex), for goalcup-verify
#                              --dev-key. Default: the key in the run's receipt.
#   MATRIX_BIN_DIR             where goalcup, goalcup-daemon and goalcup-verify are.
#   MATRIX_FETCH_INDEX         for a long run: the chunk index fetch-run.sh --index reads (the JSON
#                              its query returns), instead of DATABASE_URL.
#   MATRIX_FETCH_STORE         for a long run: a mirror of the bucket (fetch-run.sh --store),
#                              instead of CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN.
#   DATABASE_URL, CLOUDFLARE_ACCOUNT_ID, CLOUDFLARE_API_TOKEN
#                              read-only credentials for fetch-run.sh, as it documents. They leave
#                              this script's environment at once: only fetch-run.sh gets them, in
#                              its environment, never on a command line.
#   MATRIX_FETCH_BUCKET        the R2 bucket (default goalcup-traces for production,
#                              goalcup-traces-staging for the staging Worker; for any other
#                              server it is required: the row is NOT-VERIFIED without it).
#   MATRIX_FETCH_TIMEOUT       seconds the fetch may take (default 1800).
#   MATRIX_FETCH_SCRIPT        another fetch script with fetch-run.sh's arguments (tests).
#
# Options:
#   --allow-production   permit https://ingest.agentcommunity.org. Tests never use it.
#   --keep               do not delete the temporary directory (it holds the run's trace).
#   --log FILE           where the details go (default: a new file in $TMPDIR).
#   --timeout SECONDS    give up on the run after this long (default 1800; with --duration,
#                        the duration plus 1800). One budget: for a test run the seal, the end of
#                        `goalcup start` and any `goalcup upload` take what is left after the
#                        duration.
#   --test --duration SECONDS
#                        an organizer test run (`goalcup start --harness <id> --test`, approved
#                        through the same test key flow; the member must be a test operator)
#                        instead of a practice run, ended by this script: SECONDS (1 to 604800)
#                        after the prompt is submitted, or when
#                        the harness exits if that is earlier, it runs `goalcup seal <entry>` for
#                        this run's own entry. The entry is the one entry of the one run record
#                        this start wrote (a test run, of this connector), and `goalcup entries
#                        --json` must list it with this connector; anything else FAILs and nothing
#                        is sealed (never a guess). Then `goalcup start` finishes its wrap-up; if
#                        it did not report the seal uploaded, `goalcup upload <entry>` runs. The
#                        checks are the practice ones, except that the end is the seal, not the
#                        deadline: the server's copy must equal the local chain and hold the
#                        committed end (`final`); its run `status` (`open`: a run is sealed on the
#                        server only once its deadline has passed) and `stop` are not required, the
#                        server's 168-hour deadline being far off.
#                        A run that FAILs after its entry was found and before it was sealed is
#                        sealed on the way out (best effort, logged). Both options go together,
#                        only with a real connector (not a fake) and MATRIX_CONFIG_DIR. Before the
#                        login, unless MATRIX_REPO names one, the kit makes the run's repository
#                        with the GitHub login on this machine: `gh repo create
#                        <user>/goalcup-kit-<cell>-<random> --public`, no README, so it is empty.
#                        It then makes the project folder (below) and runs `goalcup start` in it.
#                        No login found: one line says to run `gh auth login` here, and the row
#                        FAILs. The kit never deletes a repository; it prints the name at the end.
#                        The log names the mode and the duration (`mode=test duration=<N>s`). A
#                        question goalcup asks before the run's approval (a repository it cannot
#                        see) FAILs the row at once: nobody is there.
#   --refusals           only with m2-fake-tui: after the login, `goalcup start --test` is run
#                        once per refusal, in a project folder made for it, with no terminal, and
#                        each must refuse before any approval or run record, saying "Nothing was
#                        created." and leaving the folder as it was:
#                          folder-not-empty  the folder holds a README;
#                          git-present       the folder holds a .git;
#                          env-no-gitignore  the folder holds a .env.local and no .gitignore;
#                          repo-has-size     GitHub says the repository has commits (its size);
#                          readme-repo       GitHub reports size 0, but the commits call says the
#                                            repository has a commit (a README-only repository);
#                          private           GitHub does not show it (private or missing: GitHub
#                                            answers both alike);
#                          bare-name         `--repo <name>` with no owner.
#                        Each row matches one short phrase of goalcup's refusal. env-no-gitignore
#                        and readme-repo refuse for checks that need goalcup with the task 2b-CLI
#                        change (a .env* file with no .gitignore line; the commits call); on an
#                        older goalcup they FAIL with "the run's approval was asked for".
#                        GitHub is a stand-in this script serves on 127.0.0.1, reached through the
#                        CLI's test doors (GOALCUP_TEST_GITHUB_API,
#                        GOALCUP_TEST_CHAT_WITHOUT_TERMINAL), so goalcup must be a test build:
#                        `cargo build -p goalcup-cli --features test-support --target-dir
#                        target/kit-test`, then MATRIX_BIN_DIR=target/kit-test/debug. A shipped
#                        build has no such doors and refuses for want of a terminal, which FAILs
#                        the row. Every line says `refusal <kind>: PASS|FAIL`; the row's version
#                        column says `refusals`.
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
# Needs: sh, curl (7.55+), python3, script(1), mktemp; macOS or Linux (WSL2 too); gh, signed in,
# for --test (its token goes to .env.local; and the repository, unless MATRIX_REPO is given).
# It uses its own temporary configuration directory, project folder and HOME. It never reads or
# writes the real goalcup configuration directory, ~/.codex, ~/.claude or ~/.hermes. The one
# exception is MATRIX_CONFIG_DIR, a directory you name for a real harness, which also keeps your
# own HOME (the harness's isolated home is made from it).
#
# The project folder. `goalcup start` works in the folder it is run from (D71): the run's
# workspace, which must hold nothing but a .gitignore and .env files. A practice cell gets an
# empty one. A --test cell gets the starting kit a player makes: a .gitignore holding `.env*`
# and a .env.local holding `GH_TOKEN=<token>`, the agent's way to push to the run's repository: the
# token of MATRIX_GH_TOKEN_FILE, or `gh auth token` only with MATRIX_ALLOW_LOGIN_TOKEN=1 (without
# either the cell refuses to start). Made with umask 077, so mode 0600; the token goes straight
# into the file and is in no argument, variable, log or output. `goalcup start` is run
# from that folder, with `--repo` for a test run. The kit strips its machine's git login
# variables from goalcup's environment, and goalcup builds the agent's environment from nothing (D71);
# a --test cell keeps the real HOME, so goalcup's own process (not the agent) can see ~/.gitconfig. The folder is under the cell's
# temporary directory, never beside the configuration directory. The token is removed from the
# log if the agent ever printed it, and .env.local is deleted even with --keep. In a Docker cell
# `gh auth token` is the container's own login (linux-matrix.sh github); the kit never copies a
# token in from the host.

set -u

PROG=$0
HERE=$(cd "$(dirname "$PROG")" && pwd) || exit 3
REPO=$(dirname "$HERE")

PRODUCTION_HOST="ingest.agentcommunity.org"
# The only servers the approval key may be sent to: the staging Worker and this machine
# (http://localhost, 127.0.0.1 or [::1], with an optional :port). Any other server is refused unless
# MATRIX_ALLOW_SERVER names it exactly (an allowlist: a denylist of production alone sends the key
# to whatever host was typed).
STAGING_SERVER="https://goalcup-ingest-staging.b-9a5.workers.dev"
ALLOW_SERVER=${MATRIX_ALLOW_SERVER:-}
unset MATRIX_ALLOW_SERVER
FAKE_ID="m2-fake-tui"
# The organisers' practice repository: the server is told it for every practice run (D69), and
# no test run may name it.
PRACTICE_REPO="agentcommunity/goalcup-matrix-empty"
# A repository name as GitHub reads it, for comparing: lowercase, no trailing / and no .git.
repo_key() {
  printf '%s' "$1" | tr 'A-Z' 'a-z' | sed -e 's:/*$::' -e 's:\.git$::' -e 's:/*$::'
}
# The refusals --refusals checks, in order.
REFUSALS="folder-not-empty git-present env-no-gitignore repo-has-size readme-repo private bare-name"

usage() {
  # The whole header: every comment line after the first, up to the first line that is not one.
  awk 'NR > 1 && !/^#/ { exit } NR > 1 { print }' "$PROG" | sed 's/^# \{0,1\}//' >&2
  exit 2
}
# A bad command line: one line naming what is wrong, not the whole header.
bad_usage() { # what
  echo "matrix: $1; see --help" >&2
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
TIMEOUT_SET=0
TEST=0
DURATION=""
REFUSE=0
while [ $# -gt 0 ]; do
  case $1 in
    -h | --help) usage ;;
    --allow-production) ALLOW_PRODUCTION=1 ;;
    --keep) KEEP=1 ;;
    --test) TEST=1 ;;
    --refusals) REFUSE=1 ;;
    --duration)
      [ $# -ge 2 ] || bad_usage "--duration needs a number of seconds"
      DURATION=$2
      shift
      ;;
    --check-copy)
      [ $# -ge 2 ] || bad_usage "--check-copy needs a configuration directory"
      CHECK_CFG=$2
      shift
      case ${2:-} in "" | -*) ;; *)
        CHECK_RUN=$2
        shift
        ;;
      esac
      ;;
    --log)
      [ $# -ge 2 ] || bad_usage "--log needs a file"
      LOG=$2
      shift
      ;;
    --timeout)
      [ $# -ge 2 ] || bad_usage "--timeout needs a number of seconds"
      TIMEOUT=$2
      TIMEOUT_SET=1
      shift
      ;;
    -*) bad_usage "unknown option $1" ;;
    *)
      if [ -z "$CONNECTOR" ]; then
        CONNECTOR=$1
      elif [ -z "$SERVER" ]; then
        SERVER=$1
      else
        bad_usage "unexpected argument $1 (the arguments are <connector> <server>)"
      fi
      ;;
  esac
  shift
done
if [ -n "$CHECK_CFG" ]; then
  [ -z "$CONNECTOR" ] || bad_usage "--check-copy takes no connector or server"
  CONNECTOR=check-copy
else
  [ -n "$CONNECTOR" ] && [ -n "$SERVER" ] || bad_usage "a connector and a server are needed"
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
# MATRIX_CODES_FILE (linux-matrix.sh sets it in a Docker cell, which has no key at all): each
# device code goalcup prints is appended to it as `<phase> <code>` (login, then start) for the host
# to approve with the key; this script approves nothing then.
CODES_FILE=${MATRIX_CODES_FILE:-}
unset MATRIX_CODES_FILE
# A --test cell's token file (project_token): the path leaves the environment too.
TOKEN_FILE=${MATRIX_GH_TOKEN_FILE:-}
unset MATRIX_GH_TOKEN_FILE
# Only an explicit 1 lets a --test cell hand the agent the whole gh login (see the header).
ALLOW_LOGIN_TOKEN=0
[ "${MATRIX_ALLOW_LOGIN_TOKEN:-}" != 1 ] || ALLOW_LOGIN_TOKEN=1
unset MATRIX_ALLOW_LOGIN_TOKEN
if [ -n "$KEYFILE" ] && { [ ! -f "$KEYFILE" ] || [ ! -r "$KEYFILE" ]; }; then
  echo "matrix: GOALCUP_TEST_APPROVAL_KEY_FILE must name a regular file this user can read" >&2
  exit 2
fi
# Not inside this checkout (a Docker cell mounts the checkout for its agent, and a key in a repository
# is easily committed), by whatever path or symlink.
if [ -n "$KEYFILE" ]; then
  key_real=$(python3 -I -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$KEYFILE" 2>/dev/null) || {
    echo "matrix: cannot resolve the path of GOALCUP_TEST_APPROVAL_KEY_FILE (python is needed)" >&2
    exit 2
  }
  key_here=$(cd "$REPO" && pwd -P)
  case $key_real in
    "$key_here"/*)
      echo "matrix: GOALCUP_TEST_APPROVAL_KEY_FILE ($KEYFILE) is inside the checkout $key_here: keep it outside" >&2
      exit 2
      ;;
  esac
fi
# The read-only credentials of the long-run fetch leave the environment too: goalcup, the daemon
# and the harness never inherit them; only fetch-run.sh gets them (fetch_copy).
FETCH_DB=${DATABASE_URL:-}
FETCH_CF_ACCOUNT=${CLOUDFLARE_ACCOUNT_ID:-}
FETCH_CF_TOKEN=${CLOUDFLARE_API_TOKEN:-}
unset DATABASE_URL CLOUDFLARE_ACCOUNT_ID CLOUDFLARE_API_TOKEN
FETCH_INDEX=${MATRIX_FETCH_INDEX:-}
FETCH_STORE=${MATRIX_FETCH_STORE:-}
FETCH_SCRIPT=${MATRIX_FETCH_SCRIPT:-$HERE/fetch-run.sh}
FETCH_TIMEOUT=${MATRIX_FETCH_TIMEOUT:-1800}
case $FETCH_TIMEOUT in '' | *[!0-9]*) echo "matrix: MATRIX_FETCH_TIMEOUT is a number of seconds" >&2; exit 2 ;; esac
PRE_KEYS=${MATRIX_PRE_PROMPT_KEYS:-}
case $PRE_KEYS in
  *\'* | *%* | *\$* | *\`*)
    echo "matrix: MATRIX_PRE_PROMPT_KEYS may hold no quote, %, \$ or backquote" >&2
    exit 2
    ;;
esac
PRE_DELAY=${MATRIX_PRE_PROMPT_DELAY:-5}
EXPECT_FILES=${MATRIX_EXPECT_FILES:-1}
case $EXPECT_FILES in
  0 | 1) ;;
  *)
    echo "matrix: MATRIX_EXPECT_FILES is 0 or 1" >&2
    exit 2
    ;;
esac
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
TEST_REPO=${MATRIX_REPO:-}
REPO_MADE=""

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

# --refusals: the fake harness, a configuration of its own, and nothing else.
if [ "$REFUSE" = 1 ]; then
  if [ -n "$CHECK_CFG" ] || [ "$TEST" = 1 ] || [ -n "$DURATION" ]; then
    echo "matrix: --refusals goes alone (not with --test, --duration or --check-copy)" >&2
    exit 2
  fi
  if [ "$CONNECTOR" != "$FAKE_ID" ]; then
    echo "matrix: --refusals runs with $FAKE_ID only (no harness is started)" >&2
    exit 2
  fi
  if [ -n "${MATRIX_CONFIG_DIR:-}" ]; then
    echo "matrix: --refusals makes its own configuration; unset MATRIX_CONFIG_DIR" >&2
    exit 2
  fi
fi

# --test and --duration: a test run of a real harness, in a prepared configuration directory,
# sealed by this script. Never one without the other: a test run left to the server lasts 168 h.
if [ "$TEST" = 1 ] || [ -n "$DURATION" ]; then
  if [ -n "$CHECK_CFG" ]; then
    echo "matrix: --test and --duration are not for --check-copy" >&2
    exit 2
  fi
  if [ "$TEST" != 1 ] || [ -z "$DURATION" ]; then
    echo "matrix: --test and --duration go together (a test run lasts 168 hours on the server; the script seals it after --duration SECONDS)" >&2
    exit 2
  fi
  case $DURATION in
    *[!0-9]* | "" | 0*)
      echo "matrix: --duration is a whole number of seconds from 1 to 604800" >&2
      exit 2
      ;;
  esac
  if [ "${#DURATION}" -gt 6 ] || [ "$DURATION" -gt 604800 ]; then
    echo "matrix: --duration is a whole number of seconds from 1 to 604800" >&2
    exit 2
  fi
  case $CONNECTOR in
    "$FAKE_ID")
      echo "matrix: --test needs a real connector, not $CONNECTOR" >&2
      exit 2
      ;;
  esac
  if [ -z "${MATRIX_CONFIG_DIR:-}" ]; then
    echo "matrix: --test needs MATRIX_CONFIG_DIR (a prepared configuration directory whose harness is signed in)" >&2
    exit 2
  fi
  # A test run asks for its own repository at the start (D69): the kit makes one, or MATRIX_REPO
  # names it.
  # The practice repository is refused however it is written: GitHub reads the name without case,
  # with or without a trailing .git, and with or without a trailing /.
  if [ -n "$TEST_REPO" ] && [ "$(repo_key "$TEST_REPO")" = "$PRACTICE_REPO" ]; then
    echo "matrix: MATRIX_REPO is the organisers' practice repository; a test run needs a repository of its own" >&2
    exit 2
  fi
  case $TEST_REPO in
    "") ;;
    */*/* | /* | */ | *[!A-Za-z0-9._/-]*)
      echo "matrix: MATRIX_REPO is <owner>/<name>" >&2
      exit 2
      ;;
    */*) ;;
    *)
      echo "matrix: MATRIX_REPO is <owner>/<name>" >&2
      exit 2
      ;;
  esac
  CELL=${MATRIX_CELL:-$CONNECTOR}
  case $CELL in
    *[!a-z0-9-]* | "" | -*)
      echo "matrix: MATRIX_CELL may hold only a-z, 0-9 and - (and not start with -)" >&2
      exit 2
      ;;
  esac
  [ "${#CELL}" -le 40 ] || { echo "matrix: MATRIX_CELL is at most 40 characters" >&2; exit 2; }
  # The agent is given a token it can read and could publish: never the owner's whole login by
  # default. A fine-grained token file, or an explicit say-so.
  if [ -n "$TOKEN_FILE" ] && [ -z "$TEST_REPO" ]; then
    echo "matrix: MATRIX_GH_TOKEN_FILE needs MATRIX_REPO=<owner>/<name>: a repository under the token's resource owner, new, empty and public (the kit would make one with your gh login, which the token cannot push to)" >&2
    exit 2
  fi
  if [ -z "$TOKEN_FILE" ] && [ "$ALLOW_LOGIN_TOKEN" != 1 ]; then
    echo "matrix: a --test cell gives its agent a GitHub token: set MATRIX_GH_TOKEN_FILE=<file> (a fine-grained token for a throwaway account or organisation, Contents read and write only, mode 0600), or MATRIX_ALLOW_LOGIN_TOKEN=1 to give it \`gh auth token\`, the whole login (every scope, every repository)" >&2
    exit 2
  fi
  [ "$TIMEOUT_SET" = 1 ] || TIMEOUT=$((DURATION + 1800))
fi

# The prompt from a file: checked here, read once the connector's mode is known.
PROMPT_FILE=${MATRIX_PROMPT_FILE:-}
if [ -n "$PROMPT_FILE" ]; then
  if [ -n "${MATRIX_PROMPT:-}" ]; then
    echo "matrix: set MATRIX_PROMPT or MATRIX_PROMPT_FILE, not both" >&2
    exit 2
  fi
  if [ ! -f "$PROMPT_FILE" ] || [ ! -r "$PROMPT_FILE" ]; then
    echo "matrix: MATRIX_PROMPT_FILE must name a regular file this user can read" >&2
    exit 2
  fi
fi

# The server: the staging Worker, a server on this machine (http:// only to localhost, 127.0.0.1 or
# [::1]), or the one https:// address MATRIX_ALLOW_SERVER names exactly. Never production without
# the flag, and MATRIX_ALLOW_SERVER does not lift that.
validate_server() {
  SERVER=$(printf '%s' "$SERVER" | sed 's:/*$::')
  SERVER_LC=$(printf '%s' "$SERVER" | tr 'A-Z' 'a-z')
  case $SERVER_LC in
    *"$PRODUCTION_HOST"*)
      # Only the production address itself, and only with the flag.
      if [ "$ALLOW_PRODUCTION" != 1 ] || { [ "$SERVER_LC" != "https://$PRODUCTION_HOST" ] && [ "${SERVER_LC#https://$PRODUCTION_HOST/}" = "$SERVER_LC" ]; }; then
        echo "matrix: refusing $PRODUCTION_HOST: tests never run against production (--allow-production overrides)" >&2
        exit 2
      fi
      return 0
      ;;
  esac
  case $SERVER_LC in
    "$STAGING_SERVER") return 0 ;;
  esac
  # A server on this machine: the WHOLE address is http://localhost, http://127.0.0.1 or
  # http://[::1] with an optional :digits, nothing else (no user@, no path, no query).
  for local_host in localhost 127.0.0.1 "[::1]"; do
    case $SERVER_LC in
      "http://$local_host") return 0 ;;
      "http://$local_host":*)
        local_port=${SERVER_LC#"http://$local_host":}
        case $local_port in "" | *[!0-9]*) ;; *) return 0 ;; esac
        ;;
    esac
  done
  allow_lc=$(printf '%s' "$ALLOW_SERVER" | sed 's:/*$::' | tr 'A-Z' 'a-z')
  if [ -n "$allow_lc" ] && [ "$allow_lc" = "$SERVER_LC" ]; then
    case $SERVER_LC in
      https://*) return 0 ;;
      *)
        echo "matrix: MATRIX_ALLOW_SERVER names $SERVER, which is not https://; plain http is allowed only to localhost, 127.0.0.1 or [::1]" >&2
        exit 2
        ;;
    esac
  fi
  echo "matrix: refusing server $SERVER: the approval key is sent to the server, so only $STAGING_SERVER and http://localhost, http://127.0.0.1 or http://[::1] (with an optional :port) are accepted; to use another https:// server, name it exactly: MATRIX_ALLOW_SERVER=$SERVER" >&2
  exit 2
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
# The log can hold raw terminal bytes of the run: the owner's alone (0600), also when it exists.
(umask 077 && : >"$LOG") && chmod 600 "$LOG" || {
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
PROJECT=/nonexistent
ROW_PRINTED=0
ROOT=""
CFG=""
OWN_CONFIG=0
STARTED_RUN=0
# A test run: when the prompt went in, this run's own entry, and when its seal was asked for.
SUBMIT_AT=""
SEAL_ENTRY=""
SEAL_ASKED_AT=""
EARLY=0
START_LAUNCHED=0
TOKEN_PUSHED=0
# The SHA-256 of the token given to the agent (token_copy_ready); never exported (the unset drops
# an export inherited from the environment).
unset TOKEN_SHA
TOKEN_SHA=""

print_row() {
  [ "$ROW_PRINTED" = 0 ] || return 0
  ROW_PRINTED=1
  # The version line up to its version number: "codex-cli 0.156.1" stays whole, Amp's
  # "0.0.x (released <date>, 4d ago)" keeps 0.0.x, so the row does not change from day to day.
  hv=$(printf '%s\n' "$HV" | head -n 1 |
    awk '{ o = ""; for (i = 1; i <= NF; i++) { o = o (i > 1 ? "_" : "") $i; if ($i ~ /^[0-9]/) break }; print o }')
  [ -n "$hv" ] || hv=unreported
  if [ "$RESULT" = NOT-VERIFIED ]; then
    log "$FAIL_REASON"
    printf 'matrix: %s\n' "$FAIL_REASON" >&3
  elif [ "$RESULT" != PASS ] && [ -n "$FAIL_REASON" ]; then
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
    : >"$(kit_file done)" 2>/dev/null
    # A test run that failed after its entry was found and before its seal was asked for would
    # go on for 168 hours: seal that entry (and only it) first, best effort and bounded.
    if [ "$TEST" = 1 ] && [ -n "$SEAL_ENTRY" ] && [ -z "$SEAL_ASKED_AT" ]; then
      log "the test run's entry $SEAL_ENTRY was not sealed; sealing it now (best effort, at most 120s)"
      run_bounded seal 120 "seal --yes $SEAL_ENTRY"
      src=$?
      log "goalcup seal $SEAL_ENTRY on the way out: exit $src"
      [ "$src" = 0 ] || log "seal it by hand: GOALCUP_CONFIG_DIR=$CFG goalcup seal --yes $SEAL_ENTRY"
      log_file "goalcup seal (output)" "$ROOT/seal.out"
      log_file "goalcup seal (stderr)" "$ROOT/seal.err"
    elif [ "$TEST" = 1 ] && [ -n "$SEAL_ENTRY" ] && [ ! -s "$ROOT/seal.rc" ]; then
      # A seal was asked for and is still running: let it finish (bounded) rather than kill it.
      log "goalcup seal $SEAL_ENTRY is still running; waiting for it (at most 120s)"
      cw=0
      while [ ! -s "$ROOT/seal.rc" ] && [ "$cw" -lt 120 ]; do
        sleep 2
        cw=$((cw + 2))
      done
      if [ -s "$ROOT/seal.rc" ]; then
        log "goalcup seal $SEAL_ENTRY on the way out: exit $(cat "$ROOT/seal.rc")"
      else
        log "goalcup seal $SEAL_ENTRY did not finish; seal it by hand: GOALCUP_CONFIG_DIR=$CFG goalcup seal --yes $SEAL_ENTRY"
        printf 'matrix: the test run is not sealed; seal it by hand: GOALCUP_CONFIG_DIR=%s goalcup seal --yes %s\n' "$CFG" "$SEAL_ENTRY" >&3
      fi
      log_file "goalcup seal (output)" "$ROOT/seal.out"
      log_file "goalcup seal (stderr)" "$ROOT/seal.err"
    elif [ "$TEST" = 1 ] && [ -z "$SEAL_ENTRY" ] && [ "$START_LAUNCHED" = 1 ]; then
      log "a test run may still be open on the server, its entry unknown here: in $CFG run goalcup entries, then goalcup seal --yes <entry>"
    fi
    # Whatever is still running from this script's own temporary directory, by that unique path.
    for pidfile in "$ROOT/login.pid" "$ROOT/start.pid" "$ROOT/entries.pid" "$ROOT/seal.pid" "$ROOT/upload.pid"; do
      if [ -f "$pidfile" ] && [ ! -f "$ROOT/$(basename "$pidfile" .pid).rc" ]; then kill -TERM "$(cat "$pidfile")" 2>/dev/null; fi
    done
    if command -v pkill >/dev/null 2>&1; then
      pkill -TERM -f "$ROOT/" 2>/dev/null
      # The daemon this run started from its own configuration directory, and no other.
      if [ "$STARTED_RUN" = 1 ]; then pkill -TERM -f "goalcup-daemon --config ${CFG:-/nonexistent}/" 2>/dev/null; fi
    fi
    sleep 1
    check_token_pushed
    # The scrub finds the token by its value only in a copy that is still the token given to the
    # agent (made again from its source if not); otherwise by its shape alone, and the log says so.
    if [ "$TEST" = 1 ] && [ -n "$TOKEN_SHA" ] && ! token_copy_ready; then
      rm -rf "$ROOT/private/gh-token"
      log "the token's private copy is gone or changed and could not be made again: scrub by shape only"
    fi
    scrub_token
    if [ "$KEEP" = 1 ]; then
      log "kept $ROOT (the project's .env.local is deleted)"
    else
      rm -rf "$ROOT" 2>/dev/null
      # The daemon's roots sit beside the configuration directory: ours only if ours.
      if [ "${OWN_CONFIG:-0}" = 1 ]; then rm -rf "$CFG-artifacts" "$CFG-runs" 2>/dev/null; fi
    fi
  }; elif [ "$TEST" = 1 ] && [ "$START_LAUNCHED" = 1 ]; then
    # The agent can reach the kit's temporary directory: gone, the push check cannot run.
    RESULT=FAIL
    FAIL_REASON="push check not run: the kit's temporary directory $ROOT is gone; check ${TEST_REPO:-the repository} by hand, and revoke the token if in doubt"
    TOKEN_PUSHED=1
    printf 'matrix: *** %s ***\n' "$FAIL_REASON" >&3
  fi
  if [ -n "$REPO_MADE" ]; then
    log "test repository made by the kit: $REPO_MADE"
    # Whose token the agent was given: a file's (MATRIX_GH_TOKEN_FILE), the cell's own login in a
    # Docker cell, or this machine's gh token on a host.
    # (The kit makes a repository only for a cell that was given the login's token.)
    if [ -n "$CODES_FILE" ]; then
      who="the login stored in this cell gave its agent its token"
    else
      who="this machine's gh token was given to the agent, so the login it came from can push"
    fi
    printf 'matrix: test repository made by the kit (delete it when done: first `gh auth refresh -s delete_repo`, which the default gh login lacks, then `gh repo delete %s --yes`, then `gh auth refresh -r delete_repo` to drop the scope again: %s): %s\n' "$REPO_MADE" "$who" "$REPO_MADE" >&3
  fi
  print_row
  # A pushed token is a failed cell whatever else happened.
  [ "$TOKEN_PUSHED" = 0 ] || exit 1
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

# A kit file's path under $ROOT, made fresh for a write: whatever is there (a file, or a link or a
# folder the agent planted, since it runs as this user and can reach $ROOT) is removed first. Once
# $ROOT exists the script runs with noclobber (set -C), so `>"$(kit_file NAME)"` creates the file
# with O_EXCL and never follows a link that appears in between.
kit_file() { # name
  [ -n "$ROOT" ] || { printf '/nonexistent/%s\n' "$1"; return 1; }
  rm -rf "$ROOT/$1" 2>/dev/null
  printf '%s\n' "$ROOT/$1"
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

# The phase the approvals belong to: login, then start.
PHASE=login

# Approve the first user code goalcup prints in the log $1, and no other: one code per phase (the
# login, the run). seen.txt is emptied at the start of each phase. With a key file the approval
# goes through the test route, the key piped from the file straight into curl's standard input as
# a header file, so it is in no variable, argument, environment or temp file. Without one, say what
# goalcup printed, once.
scan_and_approve() {
  [ ! -s "$ROOT/seen.txt" ] || return 0
  code=$(codes_in "$1" | head -1)
  [ -n "$code" ] || return 0
  printf '%s\n' "$code" >"$(kit_file seen.txt)"
  if [ -n "$CODES_FILE" ]; then
    printf '%s %s\n' "$PHASE" "$code" >>"$CODES_FILE" || {
      log "cannot write the device code to MATRIX_CODES_FILE"
      return 1
    }
    log "the $PHASE device code $code is written for the host to approve"
  elif [ -n "$KEYFILE" ]; then
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
# GOALCUP_NO_BROWSER keeps goalcup from opening a browser page for each approval: the script
# approves through the test route or prints the address for you. (GOALCUP_TEST_NO_BROWSER is the
# same switch for a build with test-support.)
#
# The kit strips this machine's git login variables from goalcup's environment (SSH agent and
# askpass, tokens, git and gh configuration variables, including every GIT_CONFIG_KEY_n and
# GIT_CONFIG_VALUE_n present). Goalcup itself builds the agent's environment from nothing: its own
# HOME, GIT_CONFIG_NOSYSTEM=1 and a no-key ssh command (D71), so the agent gets only the token in
# the project's .env.local. What the kit does not hide from goalcup's own process: a --test cell
# runs with the real HOME (MATRIX_CONFIG_DIR), so ~/.gitconfig and ~/.config/gh are visible to
# goalcup, not to the agent. A test run's token is a file in the project folder (project_token).
# WRAP_DIR is where the command runs: the project folder for `goalcup start`, the neutral
# $RUNS for the others. WRAP_EXTRA holds further NAME=value words for one command (the
# refusals' test doors).
WRAP_EXTRA=""
WRAP_DIR=""
write_wrapper() { # file name "goalcup arguments" [stty]
  # Always a path under $ROOT: fresh, and written with noclobber (kit_file).
  rm -rf "$1" 2>/dev/null
  {
    echo '#!/bin/sh'
    [ -z "${4:-}" ] || echo 'stty cols 120 rows 40 2>/dev/null'
    echo "rm -rf $(shq "$ROOT/$2.pid") && set -C && echo \$\$ >$(shq "$ROOT/$2.pid") || exit 1"
    echo "cd $(shq "${WRAP_DIR:-$RUNS}") || exit 1"
    # This machine's git login variables: SSH agent and askpass, keyring, tokens, git/gh configuration.
    printf 'exec env -u SSH_AUTH_SOCK -u SSH_AGENT_PID -u SSH_ASKPASS -u XDG_RUNTIME_DIR -u XDG_CONFIG_HOME'
    printf ' -u DBUS_SESSION_BUS_ADDRESS -u GH_TOKEN -u GITHUB_TOKEN -u GH_ENTERPRISE_TOKEN'
    printf ' -u GITHUB_ENTERPRISE_TOKEN -u GH_HOST -u GH_CONFIG_DIR -u GIT_CONFIG_GLOBAL -u GIT_CONFIG_SYSTEM'
    printf ' -u GIT_CONFIG_NOSYSTEM -u GIT_CONFIG_COUNT -u GIT_CONFIG_PARAMETERS -u GIT_ASKPASS -u GIT_SSH'
    printf ' -u GIT_SSH_COMMAND'
    # Every GIT_CONFIG_KEY_n / GIT_CONFIG_VALUE_n present, whatever n (names only are read).
    for gv in $(env | grep -E '^GIT_CONFIG_(KEY|VALUE)_[0-9]+=' | cut -d= -f1); do printf ' -u %s' "$gv"; done
    printf ' HOME=%s GOALCUP_CONFIG_DIR=%s' "$(shq "$HOME_DIR")" "$(shq "$CFG")"
    [ -z "$CONNECTORS_ENV" ] || printf ' GOALCUP_CONNECTORS_DIR=%s' "$(shq "$CONNECTORS_ENV")"
    printf ' GOALCUP_NO_BROWSER=1 GOALCUP_TEST_NO_BROWSER=1 PATH=%s TERM=%s' "$(shq "$HARNESS_PATH")" "$MATRIX_TERM"
    [ -z "$WRAP_EXTRA" ] || printf ' %s' "$WRAP_EXTRA"
    printf ' %s %s\n' "$(shq "$GOALCUP")" "$3"
  } >"$1"
}

# The project folder `goalcup start` is run from: $ROOT/projects/<name>, so it is no sibling of
# the configuration directory or of its -artifacts and -runs folders (the daemon refuses a workspace
# beside them). Mode 0700. Sets PROJECT; the kit files are made by project_kit and project_token.
new_project() { # name
  PROJECT="$ROOT/projects/$1"
  (umask 077 && mkdir -p "$PROJECT") || fail "cannot make the project folder $PROJECT" || return 1
}

# The starting kit's .gitignore: every .env* file is covered by a line.
project_gitignore() {
  (umask 077 && printf '.env*\n' >"$PROJECT/.gitignore") ||
    fail "cannot write $PROJECT/.gitignore" || return 1
}

# A .env.local holding `GH_TOKEN=<token>`: mode 0600 (umask 077), the token going into the file and
# nowhere else (never an argument, a variable or the log), and checked without being read out: one
# non-empty plain value. The token comes from MATRIX_GH_TOKEN_FILE when it is set; from
# `gh auth token` (the whole login: see the header) only with MATRIX_ALLOW_LOGIN_TOKEN=1; with
# neither there is no token and no run. A private copy is kept under $ROOT/private (mode 0600,
# deleted at cleanup) for the push check and the log scrub, with its SHA-256 in this shell
# (TOKEN_SHA, never exported): the agent can reach $ROOT, so a copy that is gone or rewritten is
# made again from the source at the check (token_copy_ready). The log names the source, never the value.
project_token() {
  mkdir -p "$ROOT/private" && chmod 700 "$ROOT/private" ||
    fail "cannot make $ROOT/private" || return 1
  if [ -n "$TOKEN_FILE" ]; then
    project_token_from_file || return 1
  else
    [ "$ALLOW_LOGIN_TOKEN" = 1 ] || fail "no token source (MATRIX_GH_TOKEN_FILE or MATRIX_ALLOW_LOGIN_TOKEN=1)" || return 1
    command -v gh >/dev/null 2>&1 ||
      fail "gh is needed (https://cli.github.com): the test run's agent pushes with its token, from the project's .env.local (or give MATRIX_GH_TOKEN_FILE)" || return 1
    (
      umask 077
      { printf 'GH_TOKEN='; gh auth token 2>"$(kit_file gh-token.err)"; } >"$PROJECT/.env.local"
    ) 2>>"$LOG"
    token_plain "$PROJECT/.env.local" ||
      { rm -f "$PROJECT/.env.local"; fail "no GitHub login found here: run \`gh auth login\` (HTTPS, and Yes to authenticate Git) in this environment, then run again (its token goes to the project's .env.local), or give MATRIX_GH_TOKEN_FILE" || return 1; }
    say "WARNING: the agent is given the ACCOUNT-WIDE gh login token (MATRIX_ALLOW_LOGIN_TOKEN=1): every scope, every repository of this login. A fine-grained token in MATRIX_GH_TOKEN_FILE is safer."
  fi
  (umask 077 && cp "$PROJECT/.env.local" "$ROOT/private/gh-token") ||
    fail "cannot keep a private copy of the token for the push check and the log scrub" || return 1
  TOKEN_SHA=$(token_digest "$ROOT/private/gh-token")
  [ -n "$TOKEN_SHA" ] || fail "cannot read back the private copy of the token" || return 1
  log "project folder $PROJECT: .gitignore and .env.local (GH_TOKEN, mode 0600; never printed)"
}

# The SHA-256 of the token in a GH_TOKEN=<value> file (nothing when there is none): what the
# private copy is checked against. Prints only the digest.
token_digest() {
  python3 -I -c '
import hashlib, sys
try:
    tok = open(sys.argv[1], "rb").read().strip().split(b"=", 1)[1]
except (OSError, IndexError):
    sys.exit(1)
if tok:
    print(hashlib.sha256(tok).hexdigest())
' "$1" 2>/dev/null
}

# The private copy as it was made, for the push check. The agent runs as this user and can reach
# $ROOT/private (../../private from its folder), so a copy that is gone or rewritten (its digest is
# not TOKEN_SHA) is made again from the token's source: the token file, or `gh auth token`.
# 1 when that cannot be done: then the check cannot run.
token_copy_ready() {
  [ -n "$TOKEN_SHA" ] || return 1
  [ "$(token_digest "$ROOT/private/gh-token")" != "$TOKEN_SHA" ] || return 0
  log "push check: the token's private copy is missing or changed; making it again from its source"
  if [ -L "$ROOT/private" ] || [ ! -d "$ROOT/private" ]; then rm -rf "$ROOT/private"; fi
  mkdir -p "$ROOT/private" && chmod 700 "$ROOT/private" || return 1
  rm -rf "$ROOT/private/gh-token"
  if [ -n "$TOKEN_FILE" ]; then
    token_file_to "$ROOT/private/gh-token" 2>/dev/null || return 1
  else
    command -v gh >/dev/null 2>&1 || return 1
    (umask 077 && set -C && { printf 'GH_TOKEN='; gh auth token 2>/dev/null; } >"$ROOT/private/gh-token") 2>/dev/null || return 1
  fi
  [ "$(token_digest "$ROOT/private/gh-token")" = "$TOKEN_SHA" ]
}

# $1 is exactly one line GH_TOKEN=<plain value>, mode 0600 (nothing is printed).
token_plain() {
  python3 -I - "$1" <<'PY'
import os, re, stat, sys
path = sys.argv[1]
text = open(path).read()
if not re.fullmatch(r"GH_TOKEN=[!#-&(-~]+\n", text):
    sys.exit(1)
if stat.S_IMODE(os.stat(path).st_mode) != 0o600:
    sys.exit(1)
PY
}

# MATRIX_GH_TOKEN_FILE into .env.local: a regular file (not a link), mode 0600, outside this
# checkout, one line `GH_TOKEN=<value>` or the value alone. Nothing of it is printed.
project_token_from_file() {
  token_file_to "$PROJECT/.env.local" 2>"$(kit_file token-file.err)" ||
    fail "$(last_line "$ROOT/token-file.err")" || return 1
  say "GitHub token for the agent: from MATRIX_GH_TOKEN_FILE $TOKEN_FILE"
}

# The token of MATRIX_GH_TOKEN_FILE, checked, into the new file $1 (GH_TOKEN=<value>, mode 0600).
# A gh login (gho_) or classic (ghp_) token is refused: it reaches every repository of its account.
token_file_to() { # dest
  python3 -I - "$TOKEN_FILE" "$REPO" "$1" <<'PY'
import os, re, stat, sys
src, repo, dest = sys.argv[1:4]
def no(why):
    sys.exit("MATRIX_GH_TOKEN_FILE %s" % why)
if os.path.islink(src) or not os.path.isfile(src):
    no("must name a regular file (not a link)")
if stat.S_IMODE(os.stat(src).st_mode) != 0o600:
    no("must be mode 0600 (chmod 600 it)")
real, here = os.path.realpath(src), os.path.realpath(repo)
if real == here or real.startswith(here + os.sep):
    no("is inside the checkout: keep it outside")
if os.path.getsize(src) > 4096:
    no("is not a token file (over 4 KiB)")
lines = [l for l in open(src).read().split("\n") if l.strip()]
if len(lines) != 1:
    no("must hold one line")
value = lines[0].strip()
if value.startswith("GH_TOKEN="):
    value = value[len("GH_TOKEN="):]
if not re.fullmatch(r"[!#-&(-~]+", value):
    no("must hold a token of visible characters, with no quote or space")
if value.startswith(("gho_", "ghp_")):
    no("holds a gh login or classic token (gho_/ghp_), which reaches every repository of its "
       "account: give a fine-grained token (github_pat_) of a throwaway owner")
fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
with os.fdopen(fd, "w") as f:
    f.write("GH_TOKEN=%s\n" % value)
PY
}

# What the starting kit holds, by name, for the "agent changed something" check.
project_kit_names() {
  ls -A "$PROJECT" 2>/dev/null >"$(kit_file kit.txt)"
}

# The token must never stay in what the kit keeps. If anything printed it (the agent's terminal),
# it is replaced where it stands in: the log and the files directly in $ROOT (the terminal's
# capture), the run folders (<config>-runs) and the harness's tracker homes under the config
# folder (native session files), the files of the last two newer than this run's start. It is
# found by its value (the private copy made when .env.local was: the agent may have deleted or
# rewritten .env.local) and by its shape (gh[opusr]_..., github_pat_...), so a missing copy still
# scrubs; terminal escape sequences and line wraps inside it are looked through (the file is then
# rewritten without its escape sequences). Chain files are never touched. The copy and .env.local
# are deleted, even with --keep. Nothing is read out.
scrub_token() {
  if [ "$TEST" = 1 ]; then
    python3 -I - "$TOKEN_SHA" "$ROOT/private/gh-token" "$ROOT/start-marker" "$LOG" "$ROOT" "$CFG-runs" "$CFG/capture-homes" <<'PY' 2>/dev/null
import hashlib, os, re, sys

want, copy, marker, log, root = sys.argv[1:6]
dirs = sys.argv[6:]
MARK = b"<GH_TOKEN removed>"
SHAPES = [rb"gh[opusr]_(?:\s*[A-Za-z0-9]){20,}", rb"github_pat_(?:\s*[A-Za-z0-9_]){20,}"]
try:
    token = open(copy, "rb").read().strip().split(b"=", 1)[1]
except (OSError, IndexError):
    token = b""
# Only the token given to the agent (a copy changed after token_copy_ready is not it).
if hashlib.sha256(token).hexdigest() != want:
    token = b""
pats = list(SHAPES)
if len(token) >= 8:
    # The token with white space allowed between its characters: a terminal wraps lines in it,
    # and the log prefixes each line.
    pats.append(rb"\s*".join(re.escape(bytes([c])) for c in token))
RX = re.compile(b"|".join(b"(?:" + p + b")" for p in pats))
# OSC, CSI, a character-set or other nF sequence (ESC ( B, as `tput sgr0` prints), a single-character one.
ESC = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[ -/]+[0-~]|\x1b[@-Z\\-_]")


def clean(data):
    new = RX.sub(MARK, data)
    plain = ESC.sub(b"", data)
    # Escape sequences between its characters hide it from the first pass: look at the text
    # without them (the file is then rewritten without its escape sequences).
    if plain != data and RX.search(plain):
        new = RX.sub(MARK, plain)
    return new


def scrub(path):
    try:
        with os.fdopen(os.open(path, os.O_RDWR | os.O_NOFOLLOW), "r+b") as f:
            data = f.read()
            new = clean(data)
            if new != data:
                f.seek(0)
                f.write(new)
                f.truncate()
    except OSError:
        pass


try:
    since = os.stat(marker).st_mtime - 2
except OSError:
    since = 0
files = [log]
files += [os.path.join(root, n) for n in os.listdir(root)]
for top in dirs:
    for d, subdirs, names in os.walk(top):
        subdirs[:] = [n for n in subdirs if n not in ("artifacts",)]
        for n in names:
            if n.startswith("chain"):
                continue
            path = os.path.join(d, n)
            try:
                st = os.lstat(path)
            except OSError:
                continue
            if st.st_mtime >= since and st.st_size <= 32 * 1024 * 1024:
                files.append(path)
for path in files:
    if os.path.isfile(path) and not os.path.islink(path):
        scrub(path)
PY
  fi
  rm -f "$ROOT/private/gh-token"
  [ "$PROJECT" = /nonexistent ] || rm -f "$PROJECT/.env.local"
}

# I1: after the seal, the token must not be in what the agent pushed. Every object of every git
# repository in the project folder (any .git, any directory holding HEAD and objects/, its objects
# a folder or a link to one: nested, bare, and submodule repositories under .git/modules; the search
# follows no link, git reads through an objects link), and of a
# fresh mirror clone of the cell's repository (public: anonymous, into $ROOT/clone), is read raw
# (`git cat-file --batch-all-objects --batch`: blobs binary or not, trees, commits of any shape,
# annotated tags and their messages, unreachable objects too; no diff, no textconv, no external
# driver, so nothing of the agent's .git/config runs) and searched for the token, its base64 forms
# (three alignments, standard and URL alphabets) and its percent-encoded forms. A hit FAILs the
# cell, loudly; the value is never printed. A project folder that is gone or a link or holds a
# name with a line break, a clone or a
# repository that cannot be read, and a token copy that cannot be made good (token_copy_ready, and
# again in the matcher itself) FAIL too: the check was not done.
check_token_pushed() {
  [ "$TEST" = 1 ] && [ "$START_LAUNCHED" = 1 ] || return 0
  leaked=""
  unread=""
  if ! token_copy_ready; then
    RESULT=FAIL
    FAIL_REASON="push check not run: the token copy is missing (the agent can reach the kit's folder, and the token could not be read again from its source); check ${TEST_REPO:-the repository} by hand, and revoke the token if in doubt"
    TOKEN_PUSHED=1
    printf 'matrix: *** %s ***\n' "$FAIL_REASON" >&3
    return 0
  fi
  # Fresh: nothing the agent may have put under $ROOT steers the clone (an insteadOf, a helper).
  rm -rf "$ROOT/clone" "$ROOT/clone-home"
  mkdir -p "$ROOT/clone-home"
  if [ -L "$PROJECT" ] || [ ! -d "$PROJECT" ]; then
    unread="$unread the project folder (it is gone or a link);"
  else
    # Listed into a variable, not a file the agent could rewrite: .git entries that are files or
    # links (a gitdir file, a link to a repository elsewhere), and every objects entry, a directory
    # or a link to one elsewhere (git reads through it), whose folder is a repository (it holds HEAD,
    # or it is a .git or *.git folder or a submodule repository under .git/modules: one of those
    # that git cannot read FAILs the check; a Git LFS store, <repository>/lfs, is none). The list
    # is split on line breaks, so a name holding one (it could hide a repository) FAILs the check.
    # Named by number, never by path (the agent names its folders: a name could be the token).
    gitdirs=$(find -P "$PROJECT" \( -name .git ! -type d -print \) -o \( \( -type d -o -type l \) -name objects -print \) 2>/dev/null) ||
      unread="$unread the project folder (it could not be listed);"
    nl='
'
    if [ -n "$(find -P "$PROJECT" -name "*$nl*" -print -quit 2>/dev/null)" ]; then
      unread="$unread the project folder (a name in it holds a line break);"
      gitdirs=""
    fi
    gn=0
    ls_ifs=$IFS
    IFS=$nl
    set -f
    for gd in $gitdirs; do
      IFS=$ls_ifs
      set +f
      case $gd in
        */objects)
          gd=${gd%/objects}
          case $gd in
            # A submodule repository's Git LFS store (no HEAD; its folder is a repository, not
            # a modules folder: a submodule named lfs is read).
            */.git/modules/*/lfs)
              if [ ! -e "$gd/HEAD" ] && [ -f "${gd%/lfs}/HEAD" ]; then
                case ${gd%/lfs} in */modules) ;; *) continue ;; esac
              fi
              ;;
            *.git | */.git/modules/*) ;;
            *) [ -f "$gd/HEAD" ] || continue ;;
          esac
          ;;
      esac
      gn=$((gn + 1))
      if [ "$gd" = "$PROJECT/.git" ]; then where="the project folder's repository"; else where="the project folder's repository #$gn"; fi
      leak_scan "$where" "$gd"
      case $? in 1) leaked="$leaked $where;" ;; 2) unread="$unread $where;" ;; esac
    done
    IFS=$ls_ifs
    set +f
  fi
  if [ -n "$TEST_REPO" ]; then
    tries=0
    until clone_err=$(leak_git clone --quiet --mirror "https://github.com/$TEST_REPO.git" "$ROOT/clone" 2>&1 >/dev/null); do
      tries=$((tries + 1))
      rm -rf "$ROOT/clone"
      if [ "$tries" -ge 3 ]; then
        log "push check: git clone of $TEST_REPO failed: $(printf '%s\n' "$clone_err" | tail -n 1)"
        unread="$unread the pushed repository $TEST_REPO (it could not be cloned);"
        break
      fi
      sleep 3
    done
    if [ -d "$ROOT/clone" ]; then
      leak_scan "the pushed repository $TEST_REPO" "$ROOT/clone"
      case $? in 1) leaked="$leaked the pushed repository $TEST_REPO;" ;; 2) unread="$unread the pushed repository $TEST_REPO (its objects could not be read);" ;; esac
    fi
    # The clone may hold the token: never kept, even with --keep.
    rm -rf "$ROOT/clone" "$ROOT/clone-home"
  fi
  if [ -n "$leaked" ]; then
    RESULT=FAIL
    FAIL_REASON="the token was pushed: revoke it now (found in:$leaked)"
    TOKEN_PUSHED=1
    printf 'matrix: *** %s ***\n' "$FAIL_REASON" >&3
  elif [ -n "$unread" ]; then
    RESULT=FAIL
    FAIL_REASON="the pushed repository could not be checked for the token (not checked:$unread); check it by hand, and revoke the token if in doubt"
    TOKEN_PUSHED=1
    printf 'matrix: *** %s ***\n' "$FAIL_REASON" >&3
  fi
}

# git with none of this machine's login or repository settings: its own HOME, no system or global
# config, no prompt, no lazy fetch, no fsmonitor or external diff, and none of git's own variables.
leak_git() {
  env -u SSH_AUTH_SOCK -u SSH_ASKPASS -u GH_TOKEN -u GITHUB_TOKEN -u GIT_ASKPASS -u XDG_CONFIG_HOME \
    -u GIT_DIR -u GIT_WORK_TREE -u GIT_COMMON_DIR -u GIT_INDEX_FILE -u GIT_OBJECT_DIRECTORY \
    -u GIT_ALTERNATE_OBJECT_DIRECTORIES -u GIT_NAMESPACE -u GIT_CONFIG -u GIT_CONFIG_PARAMETERS \
    -u GIT_CONFIG_COUNT -u GIT_SSH -u GIT_SSH_COMMAND -u GIT_PROXY_COMMAND -u GIT_EXEC_PATH \
    HOME="$ROOT/clone-home" GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
    GIT_NO_LAZY_FETCH=1 git -c credential.helper= -c core.fsmonitor= -c diff.external= \
    -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=30 "$@"
}

# Search every object of the git repository $2 (a .git directory, a gitdir file, or a bare
# repository) for the token. git's output goes straight into the matcher through a pipe (nothing on
# disk the agent could swap), followed by git's exit status on a last line the matcher checks. The
# matcher reads the private copy once and uses it only if its SHA-256 is TOKEN_SHA, passed as an
# argument (a digest, not the token; the token itself is in no argument): a copy the agent changed
# or removed after token_copy_ready is "not read", never "not found". Returns 0 when the token is
# not there, 1 when it is, 2 when the repository or the token could not be read (an empty
# repository is read: it has nothing). $1 is only a name for the log.
leak_scan() { # name gitdir
  ls_name=$1
  { leak_git --git-dir="$2" cat-file --batch-all-objects --batch </dev/null
    printf '\n--git-exit %s\n' "$?"; } 2>/dev/null |
    python3 -I -c '
import base64, hashlib, re, sys, urllib.parse
want, copy = sys.argv[1:3]
try:
    tok = open(copy, "rb").read().strip().split(b"=", 1)[1]
except (OSError, IndexError):
    tok = b""
if not tok or hashlib.sha256(tok).hexdigest() != want:
    sys.exit(4)
forms = {tok, urllib.parse.quote(tok.decode(), safe="").encode(),
         urllib.parse.quote_plus(tok.decode()).encode()}
for pad in range(3):
    raw = b"\0" * pad + tok
    whole = (pad + len(tok)) // 3 * 4
    skip = (0, 2, 3)[pad]
    for enc in (base64.b64encode(raw), base64.urlsafe_b64encode(raw)):
        forms.add(enc.rstrip(b"=")[skip:whole])
forms = [f for f in forms if len(f) >= 8]
keep = max(64, max(len(f) for f in forms))
tail = b""
src = sys.stdin.buffer
while True:
    chunk = src.read(1 << 20)
    if not chunk:
        break
    data = tail + chunk
    if any(f in data for f in forms):
        sys.exit(3)
    tail = data[-keep:]
# git ended with status 0, and this is the end of what it wrote.
done = re.search(rb"\n--git-exit ([0-9]+)\n\Z", tail)
sys.exit(0 if done and done.group(1) == b"0" else 2)
' "$TOKEN_SHA" "$ROOT/private/gh-token" 2>/dev/null
  ls_rc=$?
  case $ls_rc in
    0) log "push check: $ls_name searched for the token: not found"; return 0 ;;
    3) log "push check: $ls_name searched for the token: FOUND"; return 1 ;;
    4) log "push check: $ls_name not searched: the token's private copy changed during the check" ;;
    # git's message names the path, which the agent chose: it is not logged.
    *) log "push check: $ls_name could not be read" ;;
  esac
  return 2
}

# Crash leftovers: a dead kit's temporary directory (its kit.pid names no live matrix.sh) may hold
# the token's private copy and a cell's .env.local. Deleted here, at the next start, and only those
# two things, only under this user's own goalcup-matrix.XXXXXX directories, never through a link:
# no path component under the temporary directory may be one (`projects` or `projects/<name>` as a
# link would reach outside), and find -P lists without following any. The temporary directory is
# resolved (/var is /private/var on macOS) so this run's own directory is recognised. A process
# that ps cannot describe counts as live: nothing is deleted when in doubt.
sweep_stale() {
  sw_top=$(cd -P "${TMPDIR:-/tmp}" 2>/dev/null && pwd -P) || return 0
  for sw in "$sw_top"/goalcup-matrix.??????; do
    [ -d "$sw" ] && [ ! -L "$sw" ] && [ -O "$sw" ] && [ "$sw" != "$ROOT" ] &&
      [ -f "$sw/kit.pid" ] && [ ! -L "$sw/kit.pid" ] || continue
    sw_pid=$(cat "$sw/kit.pid" 2>/dev/null)
    case $sw_pid in "" | *[!0-9]*) continue ;; esac
    if kill -0 "$sw_pid" 2>/dev/null; then
      sw_cmd=$(ps -p "$sw_pid" -o command= 2>/dev/null) || continue
      case $sw_cmd in *matrix.sh*) continue ;; esac
    fi
    swept=""
    if [ -d "$sw/private" ] && [ ! -L "$sw/private" ]; then rm -rf "$sw/private" && swept="private copy"; fi
    if [ -d "$sw/projects" ] && [ ! -L "$sw/projects" ]; then
      find -P "$sw/projects" -mindepth 2 -maxdepth 2 -type f -name .env.local >"$(kit_file sweep.txt)" 2>/dev/null
      while IFS= read -r sw_env; do
        sw_dir=${sw_env%/.env.local}
        case ${sw_dir#"$sw/projects/"} in "" | .* | */*) continue ;; esac
        [ -d "$sw_dir" ] && [ ! -L "$sw_dir" ] && [ -f "$sw_env" ] && [ ! -L "$sw_env" ] || continue
        rm -f "$sw_env" && swept="$swept .env.local"
      done <"$ROOT/sweep.txt"
      rm -f "$ROOT/sweep.txt"
    fi
    [ -z "$swept" ] || log "swept the leftovers of a dead run in $sw:$swept"
  done
}

# A test run's repository: MATRIX_REPO, or a new one the kit makes with this machine's GitHub
# login, `gh repo create <user>/goalcup-kit-<cell>-<random> --public` (no README, so it is empty).
# The kit never deletes it; its name is printed at the end.
test_repo() {
  if [ -n "$TEST_REPO" ]; then
    say "test repository: $TEST_REPO (MATRIX_REPO)"
    return 0
  fi
  command -v gh >/dev/null 2>&1 ||
    fail "gh is needed to make the test repository (https://cli.github.com), or set MATRIX_REPO" || return 1
  owner=$(gh api user --jq .login 2>"$(kit_file gh.err)" | tr -d '\r')
  case $owner in
    "" | *[!A-Za-z0-9-]*)
      fail "no GitHub login found here: run \`gh auth login\` (HTTPS, and Yes to authenticate Git) in this environment, then run again" || return 1
      ;;
  esac
  name="goalcup-kit-$CELL-$(od -An -N3 -tx1 /dev/urandom | tr -d ' \n')"
  gh repo create "$owner/$name" --public >"$(kit_file gh.out)" 2>&1 ||
    fail "gh repo create $owner/$name --public: $(last_line "$ROOT/gh.out")" || return 1
  TEST_REPO="$owner/$name"
  REPO_MADE=$TEST_REPO
  say "test repository made: $TEST_REPO (public and empty; the kit never deletes it)"
}

# Wait for the background job that writes its exit status to $1, scanning the output $2 for
# approvals meanwhile. 0 when it finished (status in $1), 1 on timeout, 2 on a refused approval,
# 3 when a step of the test run's flow failed (FAIL_REASON says which). The limit is wall-clock
# time, so a step that takes a while inside the loop still counts.
wait_job() { # rcfile outfile limit
  wj_start=$(date +%s)
  while [ ! -f "$1" ]; do
    if [ "$SCAN" = 1 ]; then scan_and_approve "$2" || return 2; fi
    after_scan "$2" || return 3
    if [ $(($(date +%s) - wj_start)) -ge "$3" ]; then return 1; fi
    sleep 2
  done
  return 0
}

# One goalcup command through its wrapper, in the background, waited for at most LIMIT seconds:
# its standard output in $ROOT/NAME.out, its standard error in $ROOT/NAME.err. Returns its exit
# status, or 124 when it ran out of time (it is then ended by its recorded process id).
run_bounded() { # name limit "goalcup arguments"
  write_wrapper "$ROOT/$1-wrapper.sh" "$1" "$3"
  rm -f "$ROOT/$1.rc"
  ( ( (sh "$ROOT/$1-wrapper.sh" >"$(kit_file "$1.out")" 2>"$(kit_file "$1.err")" </dev/null); echo $? >"$(kit_file "$1.rc")") 2>/dev/null & )
  rb_start=$(date +%s)
  while [ ! -s "$ROOT/$1.rc" ]; do
    if [ $(($(date +%s) - rb_start)) -ge "$2" ]; then
      kill -TERM "$(cat "$ROOT/$1.pid" 2>/dev/null)" 2>/dev/null
      return 124
    fi
    sleep 1
  done
  return "$(cat "$ROOT/$1.rc")"
}

# The run records this start wrote: every run.json under the configuration directory not older
# than the start's wrapper (whole seconds on some systems, so equal counts as new), newest first.
fresh_runs() {
  ls -t "$CFG"/runs/*/run.json 2>/dev/null | while IFS= read -r f; do
    if ! [ "$WRAP" -nt "$f" ]; then printf '%s\n' "$f"; fi
  done
}

# What is left of --timeout since the start began, never below zero.
budget_left() {
  bl=$((TIMEOUT - ($(date +%s) - START_AT)))
  [ "$bl" -gt 0 ] || bl=0
  printf '%s\n' "$bl"
}

# ---- a test run: find its own entry, and seal it

# This run's entry, never a guess: exactly one run record written by this start, a test run of
# this connector with exactly one entry (e + 16 hex digits), and `goalcup entries --json` lists
# that entry with this connector. Sets SEAL_ENTRY; anything else fails and nothing is sealed.
resolve_entry() {
  fresh_runs >"$(kit_file fresh.txt)"
  n=$(grep -c . "$ROOT/fresh.txt")
  [ "$n" = 1 ] ||
    fail "cannot tell this run's entry: $n run records under $CFG/runs are as new as this start (want exactly one); nothing was sealed" || return 1
  rj=$(cat "$ROOT/fresh.txt")
  facts=$(python3 -I - "$rj" "$CONNECTOR" <<'PY'
import json, re, sys
try:
    r = json.load(open(sys.argv[1]))
except (OSError, ValueError) as e:
    print("problem=the run record cannot be read (%s)" % type(e).__name__)
    sys.exit(0)
problems = []
if r.get("test") is not True:
    problems.append("it is not a test run")
if r.get("connector_id") != sys.argv[2]:
    problems.append("its connector is %r" % r.get("connector_id"))
entries = r.get("entries") or []
if len(entries) != 1:
    problems.append("it has %d entries, not one" % len(entries))
elif not re.fullmatch(r"e[0-9a-f]{16}", str(entries[0].get("entry_id"))):
    problems.append("its entry id is not e followed by 16 hex digits")
if problems:
    print("problem=" + "; ".join(problems))
else:
    print("run=" + re.sub(r"[^A-Za-z0-9_-]", "?", str(r.get("run_id"))))
    print("entry=" + entries[0]["entry_id"])
PY
  )
  problem=$(printf '%s\n' "$facts" | sed -n 's/^problem=//p')
  [ -z "$problem" ] || fail "the run record $rj is not this test run's: $problem; nothing was sealed" || return 1
  entry=$(printf '%s\n' "$facts" | sed -n 's/^entry=//p')
  case $entry in e[0-9a-f]*) ;; *) fail "no entry id in the run record $rj; nothing was sealed" || return 1 ;; esac
  run_bounded entries 120 "entries --json"
  erc=$?
  log_file "goalcup entries --json" "$ROOT/entries.out"
  log_file "goalcup entries --json (stderr)" "$ROOT/entries.err"
  [ "$erc" = 0 ] || fail "goalcup entries --json exited $erc; nothing was sealed" || return 1
  listed=$(python3 -I - "$ROOT/entries.out" "$entry" "$CONNECTOR" <<'PY'
import json, sys
try:
    items = json.load(open(sys.argv[1]))
except (OSError, ValueError):
    print("unreadable")
    sys.exit(0)
hits = [e for e in items if isinstance(e, dict) and e.get("entry_id") == sys.argv[2]] if isinstance(items, list) else []
if len(hits) != 1:
    print("absent")
elif hits[0].get("connector_id") != sys.argv[3]:
    print("another connector")
else:
    print("listed")
PY
  )
  [ "$listed" = listed ] ||
    fail "goalcup entries --json does not list entry $entry of $CONNECTOR ($listed); nothing was sealed" || return 1
  SEAL_ENTRY=$entry
  log "this test run: run $(printf '%s\n' "$facts" | sed -n 's/^run=//p'), entry $SEAL_ENTRY (from $rj, listed by goalcup entries --json)"
}

# Ask for the seal of this run's entry, in the background: `goalcup start` keeps running and
# does its own wrap-up when the daemon has stopped the harness.
request_seal() { # why
  SEAL_ASKED_AT=$(date +%s)
  say "goalcup seal $SEAL_ENTRY: $1"
  write_wrapper "$ROOT/seal-wrapper.sh" seal "seal --yes $SEAL_ENTRY"
  rm -f "$ROOT/seal.rc"
  ( ( (sh "$ROOT/seal-wrapper.sh" >"$(kit_file seal.out)" 2>"$(kit_file seal.err)" </dev/null); echo $? >"$(kit_file seal.rc)") 2>/dev/null & )
}

# Called every turn of the wait for `goalcup start`: notes when the prompt went in (and finds the
# entry at once, so a failure shows early and a later failure can still seal it), then asks for
# the seal once the duration has passed.
test_tick() {
  tt_now=$(date +%s)
  if [ -z "$SUBMIT_AT" ]; then
    [ -f "$ROOT/sent" ] || return 0
    SUBMIT_AT=$tt_now
    log "the prompt was submitted; the seal is due in ${DURATION}s"
    resolve_entry || return 1
    return 0
  fi
  [ -z "$SEAL_ASKED_AT" ] || return 0
  [ $((tt_now - SUBMIT_AT)) -ge "$DURATION" ] || return 0
  request_seal "${DURATION}s have passed since the prompt was submitted"
  return 0
}

# After `goalcup start` has ended in a test run: the seal asked for (now, if the harness ended
# first), finished within the budget, and successful.
finish_test_seal() {
  if [ -z "$SEAL_ASKED_AT" ]; then
    EARLY=1
    if [ -z "$SUBMIT_AT" ]; then
      log "goalcup start ended before the prompt was submitted"
      if resolve_entry; then
        request_seal "goalcup start ended before the prompt was submitted"
      fi
      fail "goalcup start ended before the prompt was submitted; goalcup's last words: $(last_line "$ROOT/start.out")" || return 1
    fi
    log "the harness ended $(($(date +%s) - SUBMIT_AT))s after the prompt was submitted, before the duration (${DURATION}s)"
    request_seal "the harness ended before the duration"
  fi
  left=$(budget_left)
  ws=$(date +%s)
  while [ ! -s "$ROOT/seal.rc" ]; do
    if [ $(($(date +%s) - ws)) -ge "$left" ]; then
      log_file "goalcup seal (output so far)" "$ROOT/seal.out"
      fail "goalcup seal $SEAL_ENTRY did not finish within the --timeout budget" || return 1
    fi
    sleep 2
  done
  log_file "goalcup seal $SEAL_ENTRY (output)" "$ROOT/seal.out"
  log_file "goalcup seal $SEAL_ENTRY (stderr)" "$ROOT/seal.err"
  src=$(cat "$ROOT/seal.rc")
  [ "$src" = 0 ] ||
    fail "goalcup seal $SEAL_ENTRY exited $src: $(cat "$ROOT/seal.out" "$ROOT/seal.err" | last_line /dev/stdin)" || return 1
  log "mode=test duration=${DURATION}s: entry $SEAL_ENTRY sealed, asked for $((SEAL_ASKED_AT - SUBMIT_AT))s after the prompt was submitted$([ "$EARLY" = 1 ] && echo " (the harness ended first)")"
}
after_scan() { :; }
# Whether the log being waited on is still scanned for an approval code. It stops once the
# harness is up, so nothing the harness itself prints can ever be approved.
SCAN=1

# A question `goalcup start` asks before the run's approval (crates/goalcup-cli/src/commands/
# run_repo.rs: a repository GitHub does not show, a name to type). Nobody is at the terminal to
# answer it, so the row FAILs at once, in goalcup's words, instead of at the timeout. Only
# goalcup's own output is read: the harness is not up yet.
asked_before_approval() {
  [ -z "$READY_AT" ] && [ -f "$ROOT/start.out" ] || return 0
  tr -d '\r' <"$ROOT/start.out" |
    grep -qF -e 'Press Enter when done' -e 'then press Enter' -e 'Name it' -e 'Type <your-user>/' ||
    return 0
  fail "goalcup start asked a question before the run's approval, and nobody is there to answer it: $(last_line "$ROOT/start.out"). Check that the repository exists, public and empty"
}

# --refusals: a real run's start refuses before its approval, here with no terminal and no
# harness, through the CLI's test doors. The GitHub API is a stand-in on 127.0.0.1 that knows
# three repositories (one with commits, one private, one that reports size 0 but has a commit)
# and answers 404 for any other.
run_refusals() {
  HV=refusals
  ( (python3 -I - "$ROOT/github.port" >"$(kit_file github.log)" 2>&1 <<'PY'
import http.server, json, os, sys

REPOS = {"/repos/goalcup-kit/kit-has-size": {"private": False, "size": 12},
         "/repos/goalcup-kit/kit-private": {"private": True, "size": 0},
         "/repos/goalcup-kit/kit-readme": {"private": False, "size": 0}}
# GitHub's commits call: a repository with a commit answers 200 with it; an empty one, 409.
COMMITS = {"/repos/goalcup-kit/kit-has-size/commits": [{"sha": "a" * 40}],
           "/repos/goalcup-kit/kit-readme/commits": [{"sha": "b" * 40}]}


class Api(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        if path in REPOS:
            code, doc = 200, REPOS[path]
        elif path in COMMITS:
            code, doc = 200, COMMITS[path]
        elif path.endswith("/commits") and path[:-8] in REPOS:
            code, doc = 409, {"message": "Git Repository is empty."}
        else:
            code, doc = 404, {"message": "Not Found"}
        body = json.dumps(doc).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, form, *args):
        sys.stderr.write("github stand-in: " + form % args + "\n")


server = http.server.HTTPServer(("127.0.0.1", 0), Api)
with open(sys.argv[1] + ".tmp", "w") as f:
    f.write(str(server.server_port))
os.replace(sys.argv[1] + ".tmp", sys.argv[1])
server.serve_forever()
PY
  ) & )
  rw=0
  while [ ! -s "$ROOT/github.port" ] && [ "$rw" -lt 20 ]; do
    sleep 1
    rw=$((rw + 1))
  done
  [ -s "$ROOT/github.port" ] || fail "the GitHub stand-in did not start: $(last_line "$ROOT/github.log")" || return 1
  api="http://127.0.0.1:$(cat "$ROOT/github.port")"
  failed=""
  for kind in $REFUSALS; do
    if refusal "$kind" "$api"; then
      say "refusal $kind: PASS"
    else
      say "refusal $kind: FAIL: $FAIL_REASON"
      failed="$failed $kind"
    fi
  done
  log_file "the GitHub stand-in's requests" "$ROOT/github.log"
  [ -z "$failed" ] || fail "refusals that failed:$failed (each line above says why)" || return 1
  RESULT=PASS
}

# One refusal (see the header): `goalcup start --test` run in the row's own project folder, with its
# doors and no terminal, at most 60 s. It must exit non-zero with one short phrase of its own and
# "Nothing was created.", and leave no approval asked, no run record, no run folder, and the
# project folder as it was. A project holds the kit a player has (a .gitignore and a .env.local)
# unless the row says otherwise; the .env.local holds a placeholder, never a token.
refusal() { # kind api
  doors="GOALCUP_TEST_CHAT_WITHOUT_TERMINAL=1"
  args="--repo goalcup-kit/kit-ok"
  new_project "refuse-$1" || return 1
  kit=1
  case $1 in
    folder-not-empty) printf 'readme\n' >"$PROJECT/README.md"; want="also holds README.md" ;;
    git-present) mkdir "$PROJECT/.git" && printf 'ref: refs/heads/main\n' >"$PROJECT/.git/HEAD"; want="also holds .git" ;;
    env-no-gitignore) kit=env; want="does not cover .env.local" ;;
    repo-has-size)
      doors="$doors GOALCUP_TEST_GITHUB_API=$2"
      args="--repo goalcup-kit/kit-has-size" want="already has commits \(12 KB\)"
      ;;
    # The size of a repository GitHub has just made with a README can still read 0; only the
    # commits call tells.
    readme-repo)
      doors="$doors GOALCUP_TEST_GITHUB_API=$2"
      args="--repo goalcup-kit/kit-readme" want="already has commits; name"
      ;;
    # private and missing take one path in goalcup (GitHub answers a private repository with 404
    # to a caller with no login).
    private)
      doors="$doors GOALCUP_TEST_GITHUB_API=$2"
      args="--repo goalcup-kit/kit-private" want="GitHub does not show"
      ;;
    bare-name) args="--repo kit-bare-name" want="with its owner" ;;
  esac
  case $kit in
    1) project_gitignore && printf 'GH_TOKEN=kit-refusal-placeholder\n' >"$PROJECT/.env.local" || return 1 ;;
    env) printf 'GH_TOKEN=kit-refusal-placeholder\n' >"$PROJECT/.env.local" || return 1 ;;
  esac
  # The rows that are about the folder keep what they put in it (README, .git) beside the kit.
  ls -A "$PROJECT" >"$(kit_file "refuse-$1.before")"
  WRAP_EXTRA=$doors
  WRAP_DIR=$PROJECT
  run_bounded "refuse-$1" 60 "start --harness $CONNECTOR $DEV_FLAG --test $args"
  rrc=$?
  WRAP_EXTRA=""
  WRAP_DIR=""
  said="$ROOT/refuse-$1.txt"
  cat "$ROOT/refuse-$1.out" "$ROOT/refuse-$1.err" 2>/dev/null | tr -d '\r' >"$said"
  log_file "goalcup start --test $args in $PROJECT (refusal $1, exit $rrc)" "$said"
  case $rrc in
    124) fail "goalcup start did not refuse within 60s" || return 1 ;;
    0) fail "goalcup start exited 0" || return 1 ;;
  esac
  ! grep -q 'shows the code' "$said" || fail "the run's approval was asked for" || return 1
  ! ls "$CFG"/runs/*/run.json >/dev/null 2>&1 || fail "a run record was made" || return 1
  [ -z "$(ls -A "$CFG-runs" 2>/dev/null)" ] || fail "a run folder was made in $CFG-runs" || return 1
  ls -A "$PROJECT" | cmp -s - "$ROOT/refuse-$1.before" || fail "the project folder was changed" || return 1
  grep -Eq "$want" "$said" ||
    fail "it refused for another reason: $(last_line "$said") (a shipped build has no test doors and wants a terminal: see --refusals)" || return 1
  grep -qF "Nothing was created." "$said" || fail "the refusal does not say that nothing was created" || return 1
}

# ------------------------------------------------------------------------------ the matrix

run_matrix() {
  GOALCUP=$(find_bin goalcup)
  VERIFY=$(find_bin goalcup-verify)
  DAEMON=$(find_bin goalcup-daemon)
  if [ "$REFUSE" = 1 ]; then
    # The refusals come before any launch: goalcup alone.
    [ -n "$GOALCUP" ] || fail "goalcup is needed (a test build; see --refusals)" || return 1
  else
    [ -n "$GOALCUP" ] && [ -n "$VERIFY" ] && [ -n "$DAEMON" ] ||
      fail "goalcup, goalcup-daemon and goalcup-verify are needed (cargo build --workspace, or MATRIX_BIN_DIR)" || return 1
  fi
  GOALCUP_VERSION=$("$GOALCUP" --version 2>&1 | head -1)
  log "matrix $CONNECTOR on $PLATFORM against $SERVER, $DATE"
  if [ "$TEST" = 1 ]; then
    log "mode=test duration=${DURATION}s timeout=${TIMEOUT}s (an organizer test run, sealed by this script); repository: ${TEST_REPO:-made by the kit}"
  elif [ "$REFUSE" = 1 ]; then
    log "mode=refusals ($REFUSALS)"
  else
    log "mode=practice timeout=${TIMEOUT}s; an empty project folder and no repository"
  fi
  log "goalcup: $GOALCUP ($GOALCUP_VERSION); verifier: $VERIFY; daemon: $DAEMON"
  log "key file supplied: $([ -n "$KEYFILE" ] && echo yes || echo no); member: $MEMBER"

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
  # From here every write under $ROOT goes through kit_file, with noclobber (see kit_file).
  set -C
  RUNS="$ROOT/runs"
  mkdir -p "$RUNS"
  : >"$(kit_file seen.txt)"
  # Who owns this directory (a later kit sweeps the leftovers of a dead one), and when the run began.
  echo $$ >"$(kit_file kit.pid)"
  : >"$(kit_file start-marker)"
  if [ "$TEST" = 1 ] || [ "$REFUSE" = 1 ]; then sweep_stale; fi

  if [ -n "${MATRIX_CONFIG_DIR:-}" ]; then
    CFG=${MATRIX_CONFIG_DIR%/}
    case $CFG in
      "$HOME/Library/Application Support/goalcup" | "${XDG_CONFIG_HOME:-$HOME/.config}/goalcup" | "$HOME/.config/goalcup")
        fail "MATRIX_CONFIG_DIR is the real goalcup configuration directory; make a separate one with GOALCUP_CONFIG_DIR=<dir> goalcup setup" || return 1
        ;;
    esac
    [ -f "$CFG/prep.json" ] || fail "MATRIX_CONFIG_DIR $CFG is not prepared (no prep.json): run GOALCUP_CONFIG_DIR=$CFG goalcup setup" || return 1
    HOME_DIR=$HOME
    OWN_CONFIG=0
  else
    CFG="$ROOT/cfg"
    HOME_DIR="$ROOT/home"
    mkdir -p "$HOME_DIR"
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
  fi
  MATRIX_TERM=${MATRIX_TERM:-$([ "$CONNECTOR" = "$FAKE_ID" ] && echo dumb || echo xterm-256color)}
  read_manifest || return 1
  # The default writes one file, so that check 3 sees the workspace change.
  PROMPT=${MATRIX_PROMPT:-"Goal Cup matrix check: create a file named matrix-ok.txt in this folder holding the single word ok, then reply with the single word ok."}
  if [ -n "$PROMPT_FILE" ]; then
    PROMPT=$(python3 -I - "$PROMPT_FILE" 2>"$(kit_file prompt.err)" <<'PY'
import sys
try:
    text = open(sys.argv[1], "rb").read().decode("utf-8")
except UnicodeDecodeError:
    sys.exit("it is not UTF-8")
if any((ord(c) < 32 and c not in "\t\n\r") or ord(c) == 127 for c in text):
    sys.exit("it holds a control character other than tab and newline")
# One line, as typed.
text = " ".join(text.split())
if not text:
    sys.exit("it is empty")
sys.stdout.write(text)
PY
    ) || fail "MATRIX_PROMPT_FILE $PROMPT_FILE: $(last_line "$ROOT/prompt.err")" || return 1
    log "prompt from MATRIX_PROMPT_FILE $PROMPT_FILE: ${#PROMPT} characters, typed on one line"
  fi

  if [ "$OWN_CONFIG" = 1 ]; then
    # What `goalcup setup` would have settled. goalcup keeps no GitHub key (D69).
    printf '{\n  "harness": "%s"\n}\n' "$CONNECTOR" >"$CFG/prep.json"
    chmod 600 "$CFG/prep.json"
    if [ "$IS_FAKE" = 0 ]; then
      log "note: a real harness without MATRIX_CONFIG_DIR has no login in a fresh configuration"
    fi
  fi
  # The project folder goalcup start is run from. A test run's kit holds the token, so it is
  # made first: no repository is made on a machine that cannot give the agent a token.
  if [ "$REFUSE" = 0 ]; then
    new_project run || return 1
    if [ "$TEST" = 1 ]; then
      project_gitignore || return 1
      project_token || return 1
    fi
    project_kit_names
  fi
  if [ "$TEST" = 1 ]; then test_repo || return 1; fi

  # ---- login
  STARTED_RUN=1
  say "login against $SERVER"
  write_wrapper "$ROOT/login-wrapper.sh" login "login --server $(shq "$SERVER")"
  # (Started from a subshell that exits at once, so it is not a job of this shell: ending it
  # later prints no job notice. Its status is the file the loop waits for.)
  ( ( (sh "$ROOT/login-wrapper.sh" >"$(kit_file login.out)" 2>&1); echo $? >"$(kit_file login.rc)") 2>/dev/null & )
  wait_job "$ROOT/login.rc" "$ROOT/login.out" 960
  w=$?
  log_file "goalcup login" "$ROOT/login.out"
  case $w in
    1) fail "the login was not approved in time" || return 1 ;;
    2) fail "the test approval was refused (wrong key, member, or not a test server)" || return 1 ;;
  esac
  [ "$(cat "$ROOT/login.rc")" = 0 ] ||
    fail "goalcup login exited $(cat "$ROOT/login.rc"): $(last_line "$ROOT/login.out")" || return 1

  if [ "$REFUSE" = 1 ]; then
    run_refusals
    return $?
  fi

  # ---- start --practice (or --test)
  : >"$(kit_file seen.txt)"
  PHASE=start
  WRAP="$ROOT/start-wrapper.sh"
  if [ "$TEST" = 1 ]; then RUN_FLAG="--test --repo $(shq "$TEST_REPO")"; else RUN_FLAG=--practice; fi
  START_ARGS="start --harness $CONNECTOR $DEV_FLAG $RUN_FLAG"
  WRAP_DIR=$PROJECT
  write_wrapper "$WRAP" start "$START_ARGS" stty
  WRAP_DIR=""
  # The feeder holds the terminal's input open until the run is over (an end of input would be
  # an end of the chat) and types the prompt once, when the main loop says the harness is up.
  # The status file is written by the terminal's side of the pipe, so it appears when script
  # ends, not when the feeder does.
  FEED="while [ ! -f $(shq "$ROOT/done") ]; do
  if [ -f $(shq "$ROOT/prekeys-send") ] && [ ! -f $(shq "$ROOT/prekeys-sent") ]; then
    printf '$PRE_KEYS'; : > $(shq "$ROOT/prekeys-sent")
  fi
  if [ -f $(shq "$ROOT/send") ] && [ ! -f $(shq "$ROOT/sent") ]; then
    if [ -f $(shq "$ROOT/paste") ]; then
      printf '\033[200~%s\033[201~' $(shq "$PROMPT")
    else
      printf '%s' $(shq "$PROMPT")
    fi
    sleep $SUBMIT_DELAY; printf '$PROMPT_ENTER'; : > $(shq "$ROOT/sent")
  fi
  sleep 1
  done"
  if [ "$TEST" = 1 ]; then
    say "start --test (an organizer test run; sealed ${DURATION}s after the prompt is submitted, or when the harness ends)"
  else
    say "start --practice (the run lasts the server's practice length; waiting for its deadline)"
  fi
  if [ "$OS" = macos ]; then
    ( ( (eval "$FEED") | (script -q /dev/null sh "$WRAP" >"$(kit_file start.out)" 2>&1; echo $? >"$(kit_file start.rc)") 2>/dev/null ) & )
  else
    ( ( (eval "$FEED") | (script -qec "sh $(shq "$WRAP")" /dev/null >"$(kit_file start.out)" 2>&1; echo $? >"$(kit_file start.rc)") 2>/dev/null ) & )
  fi
  READY_AT=""
  PRE_AT=""
  chat_tick() {
    if [ -n "$READY_AT" ]; then
      now=$(date +%s)
      if [ -n "$PRE_KEYS" ] && [ -z "$PRE_AT" ] && [ $((now - READY_AT)) -ge "$PRE_DELAY" ]; then
        : >"$(kit_file prekeys-send)"
        PRE_AT=$now
        log "the first-run keys are typed"
      fi
      if [ ! -f "$ROOT/send" ] && [ $((now - READY_AT)) -ge "$DELAY" ]; then
        if [ -z "$PRE_KEYS" ] || { [ -f "$ROOT/prekeys-sent" ] && [ $((now - PRE_AT)) -ge 2 ]; }; then
          # A harness that has switched bracketed paste on gets the prompt as one paste, so a
          # pty that splits a long write cannot turn it into keys and the Enter stays a submit.
          if grep -qF "$(printf '\033[?2004h')" "$ROOT/start.out" 2>/dev/null; then
            : >"$(kit_file paste)"
            log "the harness has bracketed paste on: the prompt goes as one bracketed paste"
          fi
          : >"$(kit_file send)"
        fi
      fi
    fi
    if [ -z "$READY_AT" ] && grep -q "$READY_MARK" "$ROOT/start.out" 2>/dev/null; then
      READY_AT=$(date +%s)
      SCAN=0
      log "the harness is up"
    fi
    return 0
  }
  after_scan() {
    asked_before_approval || return 1
    chat_tick
    [ "$TEST" = 0 ] || test_tick
  }
  START_LAUNCHED=1
  START_AT=$(date +%s)
  wait_job "$ROOT/start.rc" "$ROOT/start.out" "$TIMEOUT"
  w=$?
  : >"$(kit_file done)"
  log_file "goalcup start (terminal)" "$ROOT/start.out"
  case $w in
    1) fail "the run did not finish within ${TIMEOUT}s" || return 1 ;;
    2) fail "the test approval was refused (wrong key, member, or not a test server)" || return 1 ;;
    3) return 1 ;;
  esac
  START_RC=$(cat "$ROOT/start.rc")
  if [ "$TEST" = 1 ]; then
    log "goalcup start exit status: $START_RC (the harness is stopped by the seal, so a non-zero status is not itself a failure)"
    finish_test_seal || return 1
  else
    log "goalcup start exit status: $START_RC (script's status; the harness is stopped by the deadline, so a non-zero status is not itself a failure)"
  fi

  tr -d '\r' <"$ROOT/start.out" >"$(kit_file start.txt)"

  # ---- what the run left (the newest run record this start wrote)
  RUN_JSON=$(fresh_runs | head -1)
  [ -n "$RUN_JSON" ] ||
    fail "no run record under $CFG/runs; goalcup's last words: $(last_line "$ROOT/start.txt")" || return 1
  RUN_DIR=$(dirname "$RUN_JSON")
  RUN_ID=$(py_get "$RUN_JSON" run_id)
  TRACE=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["entries"][-1]["trace_dir"])' "$RUN_JSON")
  ENTRY_ID=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["entries"][-1]["entry_id"])' "$RUN_JSON")
  log "run $RUN_ID, entry $ENTRY_ID, trace $TRACE"
  # Read before the end-of-run checks, so a FAIL row still names the harness version. A long
  # run's first records are pruned on this machine: its version is read from the server's copy.
  PRUNED=0
  if [ -f "$TRACE/chain.jsonl" ]; then
    read_hv "$TRACE/chain.jsonl"
  elif ls "$TRACE"/chain.[0-9]*.jsonl >/dev/null 2>&1; then
    PRUNED=1
  fi

  # goalcup's own words about how the run ended. The `deadline:` line goes to the run's host.log
  # (the terminal belongs to the harness), and `harness exited:` and `uploaded:` to the terminal:
  # so both files are read.
  { cat "$ROOT/start.txt"; [ ! -f "$RUN_DIR/host.log" ] || tr -d '\r' <"$RUN_DIR/host.log"; } >"$(kit_file ended.txt)"

  COPY_TRIES=6
  COPY_SLEEP=5
  STRICT=1
  if [ "$TEST" = 1 ]; then
    # The end is this script's seal, not the server's deadline (168 hours away).
    [ "$ENTRY_ID" = "$SEAL_ENTRY" ] ||
      fail "the run record's last entry $ENTRY_ID is not the entry that was sealed, $SEAL_ENTRY" || return 1
    if ! grep -Eq '^uploaded:.*(the|its) seal' "$ROOT/ended.txt"; then
      left=$(budget_left)
      say "goalcup start did not report the seal uploaded; goalcup upload $SEAL_ENTRY (at most ${left}s)"
      run_bounded upload "$left" "upload $SEAL_ENTRY"
      urc=$?
      log_file "goalcup upload (output)" "$ROOT/upload.out"
      log_file "goalcup upload (stderr)" "$ROOT/upload.err"
      [ "$urc" = 0 ] || fail "goalcup upload $SEAL_ENTRY exited $urc" || return 1
      ended=$(cat "$ROOT/ended.txt" 2>/dev/null)
      { printf '%s\n' "$ended"; tr -d '\r' <"$ROOT/upload.out"; tr -d '\r' <"$ROOT/upload.err"; } >"$(kit_file ended.txt)"
    fi
    grep -Eq '^uploaded:.*(the|its) seal' "$ROOT/ended.txt" ||
      fail "neither goalcup start nor goalcup upload reported the upload finished with the seal" || return 1
    STRICT=sealed
  else
    grep -q '^deadline:' "$ROOT/ended.txt" ||
      fail "the run did not end at its deadline (no 'deadline:' line in the terminal output or host.log); goalcup's last words: $(last_line "$ROOT/ended.txt")" || return 1
  fi
  if [ "$TEST" = 0 ]; then
    grep -q '^uploaded:.*the seal' "$ROOT/ended.txt" ||
      fail "goalcup did not report the upload finished with the seal (no 'uploaded: ... and the seal' line)" || return 1
  fi
  if [ "$CONNECTOR" = "$FAKE_ID" ]; then
    grep -q 'tui: noted' "$ROOT/start.txt" || fail "the typed prompt never reached the harness" || return 1
  fi
  JUDGED=$TRACE
  if [ "$PRUNED" = 1 ]; then
    # A long run: its acknowledged parts were pruned here (by design); the server's copy is judged.
    log "the trace holds chain parts and no chain.jsonl: a long run, pruned on this machine; the checks run on the server's copy"
    if ! fetch_configured; then
      RESULT=NOT-VERIFIED
      FAIL_REASON="NOT VERIFIED: long run, server copy not fetched (give MATRIX_FETCH_INDEX or DATABASE_URL, and MATRIX_FETCH_STORE or CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN, to rebuild it with scripts/fetch-run.sh)"
      return 1
    fi
    if ! fetch_bucket; then
      RESULT=NOT-VERIFIED
      FAIL_REASON="NOT VERIFIED: long run, server copy not fetched (the R2 bucket of $SERVER is not known: name it with MATRIX_FETCH_BUCKET)"
      return 1
    fi
    fetch_copy "$ENTRY_ID" || return 1
    JUDGED=$ROOT/servercopy/$ENTRY_ID
    [ "$HV" != unreported ] || read_hv "$JUDGED/chain.jsonl"
    same_tail "$TRACE" "$JUDGED/chain.jsonl" || return 1
  fi
  [ -f "$JUDGED/chain.jsonl" ] || fail "no chain.jsonl in the trace" || return 1

  # ---- 1. the verifier, 2. the server's copy
  verify_trace "$JUDGED" || return 1
  if [ "$IS_FAKE" = 0 ]; then
    agent_worked "$JUDGED" || return 1
    if [ "$EXPECT_FILES" = 1 ]; then
      workspace_changed "$RUN_JSON" || return 1
    else
      log "MATRIX_EXPECT_FILES=0: the workspace is not required to change"
    fi
  fi
  compare_copy "$RUN_ID" "$ENTRY_ID" "$JUDGED" "$RUN_DIR/credential" "$STRICT" || return 1

  RESULT=PASS
  return 0
}

# The harness version the chain at $1 names first ("unreported" when none does), into HV.
read_hv() { # chain.jsonl
  HV=$(python3 -I - "$1" <<'PY'
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
}

# The agent answered: the research report (under the same key as verify_trace) holds a usage
# observation with output or completion tokens above 0, or a decoded assistant message. Its exit
# status is not the question here (verify_trace judged the evidence); the report must exist.
agent_worked() { # trace
  rm -f "$ROOT/research.ndjson"
  if [ -n "$SKEY" ]; then
    "$VERIFY" --research-json "$1" --output "$(kit_file research.ndjson)" --dev-key "$SKEY" 2>"$(kit_file research.err)"
  else
    "$VERIFY" --research-json "$1" --output "$(kit_file research.ndjson)" 2>"$(kit_file research.err)"
  fi
  log "goalcup-verify --research-json $1: exit $?"
  log_file "goalcup-verify --research-json (stderr)" "$ROOT/research.err"
  [ -f "$ROOT/research.ndjson" ] ||
    fail "goalcup-verify --research-json wrote no report: $(last_line "$ROOT/research.err")" || return 1
  worked=$(python3 -I - "$ROOT/research.ndjson" <<'PY'
import json, sys

OUT = ("output_tokens", "completion_tokens", "outputtokens", "completiontokens")


def has_output(o):
    if isinstance(o, dict):
        for k, v in o.items():
            if (k.lower() in OUT and isinstance(v, (int, float)) and not isinstance(v, bool)
                    and v > 0):
                return True
            if isinstance(v, (dict, list)) and has_output(v):
                return True
    elif isinstance(o, list):
        return any(has_output(v) for v in o)
    return False


usage = assistant = 0
for line in open(sys.argv[1], errors="replace"):
    try:
        o = json.loads(line)
    except ValueError:
        continue
    if not isinstance(o, dict) or o.get("type") != "observation":
        continue
    ob = o.get("observation") if isinstance(o.get("observation"), dict) else {}
    fields = ob.get("fields") if isinstance(ob.get("fields"), dict) else {}
    if ob.get("kind") == "usage" and has_output(fields):
        usage += 1
    elif ob.get("kind") == "model_response" and fields.get("role") == "assistant":
        assistant += 1
print(f"{usage} usage observations with output, {assistant} assistant messages")
PY
  )
  log "agent work in the trace: ${worked:-unreadable report}"
  case $worked in
    "0 usage observations with output, 0 assistant messages" | "")
      fail "the agent never answered the prompt (no model output in the trace: ${worked:-unreadable report})" || return 1 ;;
  esac
  return 0
}

# The agent changed its workspace (the run record's `workspace`, the project folder): it holds a
# path beyond the starting kit it began with ($ROOT/kit.txt, names; the agent's own .git folder is
# not counted), or a commit on any ref was made since the run started (committer time at or after
# the record's `started_at_unix`; a pushed commit counts too; only if the agent made a repository).
# A model call alone (a title, an answer that stalled at a permission prompt) is not work.
workspace_changed() { # run.json
  worked=$(python3 -I - "$1" "$ROOT/kit.txt" 2>"$(kit_file workspace.err)" <<'PY'
import json, os, subprocess, sys

rec = json.load(open(sys.argv[1]))
kit = set(open(sys.argv[2]).read().split("\n")) if os.path.exists(sys.argv[2]) else set()
ws, start = rec.get("workspace"), rec.get("started_at_unix")
if not isinstance(ws, str) or not ws or not isinstance(start, int):
    sys.exit("the run record names no workspace or start time")
if not os.path.isdir(ws):
    sys.exit(f"the workspace {ws} is gone")

changed = 0
for d, dirs, files in os.walk(ws):
    if d == ws:
        dirs[:] = [n for n in dirs if n != ".git"]
        names = [n for n in dirs + files if n not in kit]
    else:
        names = dirs + files
    changed += len(names)
new = 0
if os.path.isdir(os.path.join(ws, ".git")):
    out = subprocess.run(["git", "-C", ws, "log", "--all", "--format=%ct"], capture_output=True, text=True)
    # An empty repository has no commit and no ref: then git log --all lists nothing or fails.
    new = sum(1 for t in out.stdout.split() if int(t) >= start)
print(f"{changed} changed paths, {new} new commits in {ws}")
PY
  ) || fail "the workspace could not be read: $(last_line "$ROOT/workspace.err")" || return 1
  log "workspace: $worked"
  case $worked in
    "0 changed paths, 0 new commits in "*)
      fail "the agent changed nothing in its workspace (${worked#0 changed paths, 0 new commits in }: nothing but the starting kit, no commit since the start; MATRIX_EXPECT_FILES=0 for a prompt that writes nothing)" || return 1 ;;
  esac
  return 0
}

# Whether a long run's server copy can be fetched: an index (file or database) and the objects
# (a bucket mirror or R2).
fetch_configured() {
  { [ -n "$FETCH_INDEX" ] || [ -n "$FETCH_DB" ]; } &&
    { [ -n "$FETCH_STORE" ] || { [ -n "$FETCH_CF_ACCOUNT" ] && [ -n "$FETCH_CF_TOKEN" ]; }; }
}

# The R2 bucket of the server's traces: MATRIX_FETCH_BUCKET, else the one known for the staging
# Worker or production; for any other server (a local Worker, a server named with
# MATRIX_ALLOW_SERVER) it is not known and not guessed. Sets FETCH_BUCKET; 1 when unknown.
fetch_bucket() {
  FETCH_BUCKET=${MATRIX_FETCH_BUCKET:-}
  [ -z "$FETCH_BUCKET" ] || return 0
  case $SERVER_LC in
    "https://$PRODUCTION_HOST" | "https://$PRODUCTION_HOST"/*) FETCH_BUCKET=goalcup-traces ;;
    "$STAGING_SERVER") FETCH_BUCKET=goalcup-traces-staging ;;
    *) return 1 ;;
  esac
}

# Rebuild the server's copy of one entry into $ROOT/servercopy/<entry> with fetch-run.sh (read
# only), bounded by MATRIX_FETCH_TIMEOUT. The credentials reach it in its environment only.
fetch_copy() { # entry
  fc_entry=$1
  fetch_bucket || fail "the R2 bucket of $SERVER is not known: name it with MATRIX_FETCH_BUCKET" || return 1
  fc_bucket=$FETCH_BUCKET
  set -- --bucket "$fc_bucket"
  [ -z "$FETCH_INDEX" ] || set -- "$@" --index "$FETCH_INDEX"
  [ -z "$FETCH_STORE" ] || set -- "$@" --store "$FETCH_STORE"
  set -- "$@" "$fc_entry" "$ROOT/servercopy"
  log "rebuilding the server's copy of entry $fc_entry: fetch-run.sh $* (index: ${FETCH_INDEX:-the database, read only}; objects: ${FETCH_STORE:-R2, read only}; at most ${FETCH_TIMEOUT}s)"
  rm -rf "$ROOT/servercopy"
  mkdir -p "$ROOT/servercopy" || fail "cannot make $ROOT/servercopy" || return 1
  (
    if [ -n "$FETCH_DB" ]; then DATABASE_URL=$FETCH_DB; export DATABASE_URL; fi
    if [ -n "$FETCH_CF_ACCOUNT" ]; then CLOUDFLARE_ACCOUNT_ID=$FETCH_CF_ACCOUNT; export CLOUDFLARE_ACCOUNT_ID; fi
    if [ -n "$FETCH_CF_TOKEN" ]; then CLOUDFLARE_API_TOKEN=$FETCH_CF_TOKEN; export CLOUDFLARE_API_TOKEN; fi
    exec sh "$FETCH_SCRIPT" "$@"
  ) >"$(kit_file fetch.out)" 2>"$(kit_file fetch.err)" </dev/null &
  fc_pid=$!
  fc_start=$(date +%s)
  while kill -0 "$fc_pid" 2>/dev/null; do
    if [ $(($(date +%s) - fc_start)) -ge "$FETCH_TIMEOUT" ]; then
      kill -TERM "$fc_pid" 2>/dev/null
      wait "$fc_pid" 2>/dev/null
      fail "fetch-run.sh did not finish within ${FETCH_TIMEOUT}s" || return 1
    fi
    sleep 1
  done
  wait "$fc_pid"
  fc_rc=$?
  log_file "fetch-run.sh (output)" "$ROOT/fetch.out"
  log_file "fetch-run.sh (stderr)" "$ROOT/fetch.err"
  [ "$fc_rc" = 0 ] || fail "fetch-run.sh exited $fc_rc: $(last_line "$ROOT/fetch.err")" || return 1
  [ -f "$ROOT/servercopy/$fc_entry/chain.jsonl" ] ||
    fail "fetch-run.sh left no chain.jsonl for entry $fc_entry" || return 1
}

# Every chain part left on this machine is byte for byte the server copy's bytes at its offset,
# and the last one ends where the copy ends.
same_tail() { # trace copy_chain
  python3 -I - "$1" "$2" >"$(kit_file tail.out)" 2>&1 <<'PY'
import os, re, sys
trace, copy = sys.argv[1], sys.argv[2]
parts = []
for name in os.listdir(trace):
    m = re.fullmatch(r"chain\.(\d{20})\.jsonl", name)
    if m:
        parts.append((int(m.group(1)), name))
parts.sort()
size = os.path.getsize(copy)
ok = bool(parts)
end = None
with open(copy, "rb") as c:
    for start, name in parts:
        with open(os.path.join(trace, name), "rb") as f:
            data = f.read()
        c.seek(start)
        good = c.read(len(data)) == data
        ok = ok and good
        end = start + len(data)
        print(("ok    " if good else "FAIL  ") + f"local {name} ({len(data)} bytes) = server copy bytes {start}..{end}")
good = end == size
ok = ok and good
print(("ok    " if good else "FAIL  ") + f"the last local part ends where the server copy ends: local={end} server={size}")
sys.exit(0 if ok else 1)
PY
  st_rc=$?
  log_file "local chain parts against the server's copy" "$ROOT/tail.out"
  [ "$st_rc" = 0 ] || fail "the local chain parts are not the server copy's tail (see the comparison in the log)" || return 1
}

# The local verifier on one trace: exit 0, integrity valid, and the anchor signed by the server's
# acknowledgement. A server whose signing key this build does not pin (local, staging) is
# verified under a key supplied for the run (--dev-key; MATRIX_SERVER_KEY, else the key in the
# run's own receipt), and the log says so.
verify_trace() { # trace [allow_incomplete]
  "$VERIFY" --capture-integrity "$1" >"$(kit_file verify.out)" 2>"$(kit_file verify.err)"
  VRC=$?
  KEYNOTE="the pinned server key"
  VERIFY_NOTE=""
  SKEY=""
  if [ "$VRC" = 2 ] && grep -q receipt_key_not_accepted "$ROOT/verify.out"; then
    SKEY=${MATRIX_SERVER_KEY:-$(py_get "$1/receipt.json" public_key)}
    log "verify without a supplied key: exit $VRC; retrying with --dev-key (the server's own signing key is not pinned in this build for $SERVER)"
    "$VERIFY" --capture-integrity "$1" --dev-key "$SKEY" >"$(kit_file verify.out)" 2>"$(kit_file verify.err)"
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
# run must also be sealed and over. With strict=sealed (a test run this script sealed) the entry's
# committed end (`final`) must be the local chain's last record; the run's `status` and `stop` are
# not required: a run is `sealed` on the server only once its deadline has passed (db.ts statusAt,
# a test run's is 168 hours away), so a test run stays `open` after its seal. (This was F2: every
# test row failed on `status is sealed: server='open'`.)
compare_copy() { # run_id entry_id trace credential_file strict
  tries=0
  while :; do
    # The credential goes to curl as a header on its stdin; it is in no argument, log or file.
    status=$(python3 -I -c 'import json,sys; print("Authorization: Bearer " + json.load(open(sys.argv[1]))["credential"])' "$4" |
      http GET "/v1/runs/$1?entry_id=$2" "$ROOT/server.json" -H @-)
    log "GET /v1/runs/$1?entry_id=$2: HTTP $status $(cat "$ROOT/server.json" 2>/dev/null)"
    [ "$status" = 200 ] || fail "GET /v1/runs/$1 with the run's credential answered HTTP $status" || return 1
    [ "$5" != 0 ] || break
    if [ "$5" = sealed ]; then
      [ "$(py_get "$ROOT/server.json" final)" != "" ] && [ "$(py_get "$ROOT/server.json" final)" != None ] && break
    else
      [ "$(py_get "$ROOT/server.json" status)" = sealed ] && break
    fi
    tries=$((tries + 1))
    [ "$tries" -lt "${COPY_TRIES:-6}" ] || break
    sleep "${COPY_SLEEP:-5}"
  done
  python3 -I - "$ROOT/server.json" "$ROOT/verify.out" "$3/chain.jsonl" "$5" >"$(kit_file compare.out)" 2>&1 <<'PY'
import json, os, sys
server = json.load(open(sys.argv[1]))
verify = json.load(open(sys.argv[2]))
chain_bytes = os.path.getsize(sys.argv[3])
strict = sys.argv[4] in ("1", "sealed")
whole = sys.argv[4] == "1"
stop = whole
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
if whole:
    checks += [("status is sealed", server.get("status"), "sealed")]
if stop:
    checks += [("the run is over (stop)", server.get("stop"), True)]
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
  unverified=0
  n=0
  # One "entry_id=trace_dir" per line, read a line at a time so a path may hold spaces. (fd 4, so
  # nothing run inside the loop can read the list from standard input.)
  python3 -I -c 'import json,sys
for e in json.load(open(sys.argv[1]))["entries"]:
    print(e["entry_id"] + "=" + e["trace_dir"])' "$RUN_JSON" >"$(kit_file entries.txt)"
  while IFS= read -r pair <&4; do
    entry=${pair%%=*}
    trace=${pair#*=}
    say "entry $entry (segment $n)"
    if [ ! -f "$trace/chain.jsonl" ] && ls "$trace"/chain.[0-9]*.jsonl >/dev/null 2>&1; then
      # A long run's entry, pruned on this machine: its server copy is judged, if it can be fetched.
      if ! fetch_configured; then
        say "entry $entry: NOT VERIFIED: long run, server copy not fetched"
        unverified=1
      elif fetch_copy "$entry" && same_tail "$trace" "$ROOT/servercopy/$entry/chain.jsonl" &&
        verify_trace "$ROOT/servercopy/$entry" 1 &&
        compare_copy "$RUN_ID" "$entry" "$ROOT/servercopy/$entry" "$RUN_DIR/credential" 0; then
        say "entry $entry: pruned here; the server's copy verifies (verifier exit $VRC${VERIFY_NOTE:+, incomplete coverage}), matches its position, and ends with the local parts"
      else
        say "entry $entry: $FAIL_REASON"
        ok=0
      fi
    elif [ ! -f "$trace/chain.jsonl" ]; then
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
  if [ "$unverified" = 1 ]; then
    RESULT=NOT-VERIFIED
    FAIL_REASON="NOT VERIFIED: long run, server copy not fetched"
    return 1
  fi
  RESULT=PASS
}

# The last non-empty line of a file, carriage returns removed, cut to 300 characters.
last_line() { # file
  tr -d '\r' 2>/dev/null <"$1" | grep -v '^[[:space:]]*$' | tail -1 | cut -c1-300
}

# The value of a top-level key of a JSON file ("" when absent).
py_get() { # file key
  python3 -I -c 'import json,sys
try:
    v = json.load(open(sys.argv[1])).get(sys.argv[2], "")
except Exception:
    v = ""
print(v if not isinstance(v, bool) else str(v).lower())' "$1" "$2"
}

# The connector's harness.toml, found the way goalcup finds its connectors: GOALCUP_CONNECTORS_DIR,
# else the directory beside the binary, else this checkout's. A manifest nowhere is a failure.
# Every connector runs in chat mode (D57); goalcup itself refuses one that does not.
read_manifest() {
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
    fail "no harness.toml found for $CONNECTOR (set GOALCUP_CONNECTORS_DIR to the connectors directory)" || return 1
  fi
  log "connector $CONNECTOR: $manifest"
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
  } >"$(kit_file "bin/$FAKE_ID")"
  # `#!/usr/bin/env node` as an npm harness has it: the `node` found is a stand-in on the PATH
  # the tracker gives the harness, which is the participant's own.
  printf '#!/bin/sh\nexec /bin/sh "$@"\n' >"$(kit_file bin/node)"
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

if [ -n "$CHECK_CFG" ]; then run_check; else run_matrix; fi
if [ "$RESULT" != PASS ] && [ -n "${RUN_DIR:-}" ] && [ -f "$RUN_DIR/host.log" ]; then
  log_file "host.log (goalcup's messages during the run)" "$RUN_DIR/host.log"
fi
case $RESULT in
  PASS) exit 0 ;;
  NOT-VERIFIED) exit 4 ;;
  *) exit 1 ;;
esac
