#!/bin/sh
# scripts/linux-matrix.sh build [<connector>...]
# scripts/linux-matrix.sh login <connector>
# scripts/linux-matrix.sh github <connector>
# scripts/linux-matrix.sh run <connector> [-- <matrix.sh options>]
#
# The matrix (scripts/matrix.sh) for one real harness on Linux, in Docker, against staging.
# docs/OPERATING.md, "The matrix", says what a row proves.
#
# Each connector has an image with its harness installed at the version the connector's first
# line names: codex-cli from npm (@openai/codex), claude-code-cli from npm
# (@anthropic-ai/claude-code), amp-cli from npm (`npm install --prefix /opt/amp @ampcode/cli@<v>`,
# the install measured in docs/harnesses/amp-cli.md), hermes-cli from PyPI. The Linux binaries are
# built from this checkout into a Docker volume of the checkout's own
# (goalcup-linux-matrix-target-<n>) and, at every login and run, installed as packaging/install.sh
# installs them: copied into the dev user's ~/.local/bin, which is first on the container's PATH
# (/target/debug is not on it). The goalcup configuration directory, the harness's tracker home
# in it, the work folder and HOME live on a named volume of the connector's (or cell's) own,
# goalcup-linux-matrix-<connector>[-<cell>], so a harness sign-in done once survives every later
# run. `docker volume rm goalcup-linux-matrix-<connector>[-<cell>]` forgets it.
#
# build   builds the image of each connector named (default: every connector this script has an
#         install recipe for) and the Linux binaries, once, into the checkout's target volume,
#         under the heavy-run lock. Then runs need not build (LINUX_MATRIX_PREBUILT=1).
# login   `goalcup login` against the server (approved with the test key, unattended), then
#         `goalcup setup --harness <connector>` in this terminal (`docker exec -it`): the harness
#         opens in its tracker home and the owner signs in there. Without a terminal, everything
#         up to that point is done and the command to run in one is printed (exit 3). Once setup
#         has recorded the sign-in (`home_ready`), login does not open the harness again. Either
#         way it then prints the NAMES (never the contents) of the files in the tracker home, so
#         the place a harness keeps its login is on record (Claude Code's on Linux is unmeasured),
#         and writes them to login-<connector>[-<cell>]-<time>.log in the log directory.
# github  `gh auth login` in this terminal, in the container, on the cell's volume (HOME is on
#         it, so the login stays for every later run): what a test run's repository is made with
#         and its agent pushes with, as on a player's machine. Choose HTTPS and say Yes to
#         "Authenticate Git". The kit never copies or mounts this machine's login or a token.
#         Signed in already: it says as whom and opens nothing. Needs no key.
# run     scripts/matrix.sh <connector> <server> in the container, with MATRIX_CONFIG_DIR on the
#         volume. Options after `--` go to matrix.sh as they are; only --test, --duration N,
#         --timeout N and --keep are accepted (e.g. `-- --test --duration 7200`: an organizer test
#         run sealed after two hours; the member must be a test operator, and the container's
#         GitHub login, from `github`, makes the run's repository and pushes to it). Without
#         --test, a practice run: no repository and no git login. Prints the matrix row last,
#         and exits with matrix.sh's status. (matrix.sh --refusals needs no harness and a test
#         build of goalcup: run it on the host.)
#
# Owner rules (scripts/kit_settle.py, which campaign.py applies to host cells too), applied to the
# tracker home before every login and run (never ~/.codex or ~/.claude, which are not touched):
#   codex-cli        config.toml gets `model = "gpt-5.6-luna"` (never gpt-6-astra) and
#                    `model_reasoning_effort` = LINUX_MATRIX_CODEX_EFFORT. After the run, every
#                    turn of the run's new Codex sessions must say gpt-5.6-luna at that effort.
#   claude-code-cli  settings.json in its config home (CLAUDE_CONFIG_DIR, the tracker home) gets
#                    `"model": "haiku"` and `"alwaysThinkingEnabled": false`, other keys kept.
#                    After the run, every assistant message in the run's new transcripts (under
#                    the home's projects/, subagents included) must name a haiku model. Claude's
#                    own `<synthetic>` messages (no model call) are counted apart, not judged.
# A message with another model, or none found at all, fails the run.
# No permission prompts (nobody may answer one; the container is the sandbox): codex-cli also gets
# `approval_policy = "never"` and `sandbox_mode = "danger-full-access"`, claude-code-cli
# `permissions.defaultMode = "bypassPermissions"` and `"skipDangerousModePermissionPrompt": true`.
# A test-kit setting, like the cheapest model; Goal Cup never sets a participant's.
#
# goalcup never hands a harness a provider key (D61): a harness signs in with its own login,
# kept in its tracker home, as Codex does.
#
# Environment. A secret is only ever a FILE on this machine, and it is never mounted into the
# cell's container (the harness runs there, as the same user as every step, with no sandbox, so it
# could read a mounted file). Its value goes on the standard input of the one `docker exec` that
# needs it, never in an argument, a `-e`, a log or this script's output.
#   GOALCUP_TEST_APPROVAL_KEY_FILE  the server's test approval key (login and run). It is the
#                                   standard input of the login step and of the run step, which
#                                   keep it in a private file in the container's own /tmp;
#                                   matrix.sh removes the run's file once the run is approved,
#                                   before the harness works, and each step removes it on exit.
#   MATRIX_REPO                     a test run's repository, <owner>/<name>, instead of the one
#                                   matrix.sh makes (matrix.sh says what it must be).
#   GOALCUP_TEST_MEMBER_ID          the member approved as
#                                   (default 00000000-0000-4000-8000-0000000c0de1).
#   LINUX_MATRIX_SERVER             default https://goalcup-ingest-staging.b-9a5.workers.dev;
#                                   production is refused.
#   LINUX_MATRIX_CELL               a cell name ([a-z0-9-], at most 40, not starting with -) so
#                                   that cells of one connector run side by side: the volume and
#                                   the container are goalcup-linux-matrix-<connector>-<cell>
#                                   (a fixed name, so an orchestrator can `docker network
#                                   disconnect` or `docker pause` it), labelled
#                                   goalcup-linux-matrix=<connector>/<cell>. Each cell signs in
#                                   once on its own volume. Unset: one volume per connector and a
#                                   container named by this process's id, as before.
#   LINUX_MATRIX_PREBUILT=1         login and run use the image and binaries `build` made: no
#                                   cargo, no image build, no heavy-run lock, the target volume
#                                   mounted read-only (so parallel runs never build into it).
#                                   Fails when the image or the binaries are missing.
#   LINUX_MATRIX_CODEX_EFFORT       Codex's reasoning effort: low (default) or minimal. minimal
#                                   only if the installed codex lists it for gpt-5.6-luna in
#                                   `codex debug models --bundled`; otherwise the login or run
#                                   fails before the harness opens.
#   MATRIX_PROMPT_FILE              a prompt file for matrix.sh (absolute path), mounted read-only.
#   MATRIX_PROMPT, MATRIX_PROMPT_DELAY, MATRIX_SUBMIT_DELAY, MATRIX_PRE_PROMPT_KEYS,
#   MATRIX_PRE_PROMPT_DELAY, MATRIX_EXPECT_FILES
#                                   passed to matrix.sh when set (see its header). For
#                                   claude-code-cli MATRIX_PRE_PROMPT_KEYS defaults to '\033[B\r',
#                                   the answer to its "do you trust this folder" question in every
#                                   new workspace (measured on macOS); set it (even empty) to
#                                   override.
#   MATRIX_FETCH_INDEX, MATRIX_FETCH_STORE
#                                   a long run's server copy, as on a host (matrix.sh rebuilds it
#                                   with fetch-run.sh): the chunk index file and the bucket mirror
#                                   directory (absolute paths), mounted read-only.
#   MATRIX_DATABASE_URL_FILE, MATRIX_CLOUDFLARE_TOKEN_FILE
#                                   or the read-only database URL and R2 token, each a file
#                                   (absolute path) on this machine, never mounted: they are not in
#                                   the container while the harness works. When a long run needs
#                                   them, at its end, matrix.sh there makes a private directory and a
#                                   request file (MATRIX_FETCH_REQUEST, in the log directory); this
#                                   script, polling for it, puts the two in that directory in a
#                                   `docker exec` of their own (standard input), once per run, and
#                                   matrix.sh reads them, removes them at once and hands them to
#                                   fetch-run.sh alone. (The agent could make the request file too;
#                                   it then reaches only that directory, once.) DATABASE_URL or
#                                   CLOUDFLARE_API_TOKEN in this script's environment is never
#                                   passed in. CLOUDFLARE_ACCOUNT_ID, MATRIX_FETCH_BUCKET and
#                                   MATRIX_FETCH_TIMEOUT are passed on when set. The image holds zstd
#                                   and psql for fetch-run.sh.
#   LINUX_MATRIX_LOG_DIR            where matrix.sh's log lands on this machine
#                                   (default $TMPDIR/goalcup-linux-matrix).
#   GOALCUP_HEAVY_LOCK              the shared heavy-run lock (default /tmp/goalcup-heavy.lock).
#                                   The script takes it for the build (and for a login or run that
#                                   builds) and drops it before the interactive sign-in. Never run
#                                   it inside the lock: it would wait for its own caller. A wrapper
#                                   that holds the lock sets GOALCUP_HEAVY_LOCK_HELD=1, and the
#                                   script refuses at once; otherwise it waits at most
#                                   LINUX_MATRIX_LOCK_WAIT seconds (default 1800), then fails.
#
# The script has no drill of its own: an orchestrator acts on the named container from outside.
#
# What the harness can read. It runs as the same user as the script's own steps, so it can read
# everything in the container: the volume (the container's GitHub login among it, which a test run
# pushes with, D69), the prompt file, the fetch index and bucket mirror when given, and the whole
# checkout at /src, untracked files included. No key or credential file is mounted. Run it from a clean checkout, never
# one holding secrets (a Worker .dev.vars, for example).
#
# Needs: Docker (no privileged container), `cargo fetch --locked` run on this machine first (the
# container builds --offline from its registry), and network.
# Last line: `linux-matrix: <matrix row>`, `linux-matrix: FAIL <step>`, `linux-matrix: built ...`,
# or for a login, `linux-matrix: signed in` or `linux-matrix: OWNER STEP ...`.
set -u

STAGING=https://goalcup-ingest-staging.b-9a5.workers.dev
SYNTHETIC_MEMBER=00000000-0000-4000-8000-0000000c0de1
# The owner's models, for the words this script prints (scripts/kit_settle.py holds the rules).
LUNA=gpt-5.6-luna
CLAUDE_MODEL=haiku
# Paths inside the container.
VOL=/vol
CFG=$VOL/cfg
# The dev user's ~/.local/bin (HOME is $VOL/home), where the binaries are installed.
BIN=$VOL/home/.local/bin
PROMPT_IN=/run/goalcup/prompt
FETCH_INDEX_IN=/run/goalcup/fetch-index
FETCH_STORE_IN=/run/goalcup/fetch-store
# Where matrix.sh in the container asks the host for the fetch credentials, and the private
# directory it makes for the answer (inner_fetch_creds); both are removed as soon as they are used.
FETCH_SECRETS_DIR=/tmp/goalcup-fetch-secrets
# The settle helper beside this script (/src/scripts in the container).
KIT=$(dirname "$0")/kit_settle.py
# matrix.sh's settings this script passes on when they are set.
PASS_ENV="MATRIX_PROMPT MATRIX_PROMPT_DELAY MATRIX_SUBMIT_DELAY MATRIX_PRE_PROMPT_KEYS MATRIX_PRE_PROMPT_DELAY MATRIX_REPO MATRIX_EXPECT_FILES"

fail() {
    echo "linux-matrix: FAIL $*"
    exit 1
}
usage() {
    awk 'NR > 1 && !/^#/ { exit } NR > 1 { print }' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}
# A bad command line: one line naming what is wrong.
bad_usage() { # what
    echo "linux-matrix: $1; see --help" >&2
    exit 2
}

# The connector's version, from its manifest's first line (`# Codex CLI 0.153.0, ...`, or
# `# Amp 0.0.1792209635-g24a9fe, ...`): digits.digits.digits and any -suffix, up to the comma.
version_of() { # manifest
    sed -n '1s/^#[^0-9]* \([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*[-.A-Za-z0-9]*\).*/\1/p' "$1"
}

# Codex's reasoning effort, checked: low or minimal.
codex_effort() {
    e=${LINUX_MATRIX_CODEX_EFFORT:-low}
    case $e in low | minimal) printf '%s\n' "$e" ;; *) return 1 ;; esac
}

# ------------------------------------------------------------------ inside the container
# These run as `dev` in the container, called by the host half below.

# A secret from this command's standard input (the host half's `docker exec -i ... <file`) into a
# new private file in the container's own /tmp, never the volume (which outlives the container);
# its path goes into the variable named. Read before anything else runs, then standard input is
# /dev/null, so no child inherits it. Every such file is removed on the way out (the EXIT trap):
# no secret is in the container while the harness runs (a run's matrix.sh removes the key itself
# once the run is approved).
SECRET_DIRS=""
secret_in() { # variable
    sd=$(mktemp -d /tmp/goalcup-secret.XXXXXX) || fail "a private directory for a secret"
    SECRET_DIRS="$SECRET_DIRS $sd"
    (umask 077 && cat >"$sd/secret") || fail "reading a secret from standard input"
    exec </dev/null
    [ -s "$sd/secret" ] || fail "no secret on standard input"
    eval "$1=\$sd/secret"
}
forget_secrets() {
    for sd in $SECRET_DIRS; do rm -rf "$sd"; done
    SECRET_DIRS=""
}

# The tracker home's config for a harness that has a model rule (scripts/kit_settle.py
# home-config; minimal only if the installed codex lists it for the Luna model).
inner_home_config() { # connector
    effort=$(codex_effort) || fail "LINUX_MATRIX_CODEX_EFFORT must be low or minimal"
    python3 "$KIT" home-config "$CFG" "$1" "$effort" || fail "home config of $1 (left as it was)"
}

# The names (never the contents) of the files in a connector's tracker home and its run home,
# relative to them: where a harness keeps its login, on record.
inner_home_files() { # connector
    python3 - "$CFG/capture-homes/$1" "$CFG/capture-homes/$1.run-home" <<'PY'
import os, sys

for root in sys.argv[1:]:
    if not os.path.isdir(root):
        continue
    names = []
    for d, dirs, files in os.walk(root):
        dirs.sort()
        rel = os.path.relpath(d, root)
        depth = 0 if rel == "." else rel.count(os.sep) + 1
        if depth >= 4:
            dirs[:] = []
        for f in sorted(files):
            names.append(os.path.normpath(os.path.join(rel, f)))
    print(f"linux-matrix: files under {root} (names only, {len(names)}):")
    for n in names[:200]:
        print(f"linux-matrix:   {n}")
    if len(names) > 200:
        print(f"linux-matrix:   ... and {len(names) - 200} more")
PY
}

# `goalcup login`, its one device code approved through the test route: the key arrives on this
# step's standard input into a private file, and goes from it straight into curl's standard input
# as a header, as matrix.sh does it. The file is gone when this step ends.
inner_login() {
    secret_in keyfile
    mkdir -p "$CFG" && chmod 700 "$CFG" || fail "login: config directory"
    out=$TMPDIR/login.$$
    rm -f "$out" "$out.rc"
    (GOALCUP_TEST_NO_BROWSER=1 goalcup login --server "$LINUX_MATRIX_SERVER" >"$out" 2>&1; echo $? >"$out.rc") &
    job=$!
    approved="" waited=0
    while [ ! -f "$out.rc" ]; do
        code=$(tr -d '\r' <"$out" | sed -n 's/.*shows the code \([A-Z]\{4\}-[A-Z]\{4\}\)\..*/\1/p' | head -n 1)
        if [ -z "$approved" ] && [ -n "$code" ]; then
            approved=$code
            status=$({
                printf 'Authorization: Bearer '
                tr -d '\n' <"$keyfile"
                printf '\n'
            } | curl -sS --max-time 30 -o /dev/null -w '%{http_code}' -X POST -H @- \
                -H 'Content-Type: application/json' \
                -d "{\"user_code\":\"$code\",\"member_id\":\"$GOALCUP_TEST_MEMBER_ID\",\"approve\":true}" \
                "$LINUX_MATRIX_SERVER/v1/device/approve")
            echo "linux-matrix: login code approved as $GOALCUP_TEST_MEMBER_ID: HTTP $status"
            [ "$status" = 200 ] || { kill "$job" 2>/dev/null; fail "login approval (HTTP $status)"; }
        fi
        [ "$waited" -lt 300 ] || { kill "$job" 2>/dev/null; cat "$out"; fail "login timed out"; }
        sleep 2
        waited=$((waited + 2))
    done
    rc=$(cat "$out.rc")
    [ "$rc" = 0 ] || { cat "$out"; fail "goalcup login exited $rc"; }
    tail -n 1 "$out"
}

# What setup recorded (prep.json): `yes` once the owner has signed in to this connector's
# tracker home.
inner_home_ready() { # connector
    python3 - "$CFG/prep.json" "$1" <<'PY'
import json, sys

try:
    prep = json.load(open(sys.argv[1]))
except (OSError, ValueError):
    prep = {}
print("yes" if prep.get("harness") == sys.argv[2] and prep.get("home_ready") else "no")
PY
}

# After setup: the work folder a run needs (scripts/kit_settle.py settle). A test run names its
# repository at its start, and matrix.sh makes its git login; nothing about GitHub is kept here.
inner_settle() {
    mkdir -p "$VOL/work" || fail "work folder"
    python3 "$KIT" settle "$CFG" "$VOL/work" || fail "settle prep.json"
}

# The rule the run is held to, in words.
rule_of() { # connector
    case $1 in
    codex-cli) echo "every Codex turn must be $LUNA at effort $(codex_effort || echo '?')" ;;
    claude-code-cli) echo "every Claude assistant message must be a $CLAUDE_MODEL model" ;;
    esac
}

# The run's models, from what the harness itself wrote since the marker (scripts/kit_settle.py
# model-check). A connector without a rule passes; finding nothing fails: the rule cannot be
# confirmed.
inner_model_check() { # connector marker
    effort=$(codex_effort) || fail "LINUX_MATRIX_CODEX_EFFORT must be low or minimal"
    python3 "$KIT" model-check "$CFG" "$1" "$2" "$effort"
}

inner_run() { # connector [matrix.sh options]
    h=$1
    shift
    # The approval key, from this step's standard input. matrix.sh removes the file once the run
    # is approved, before the harness works (MATRIX_FORGET_KEY); the fetch credentials are not
    # here at all: matrix.sh asks the host for them when it needs them (MATRIX_FETCH_REQUEST).
    secret_in keyfile
    GOALCUP_TEST_APPROVAL_KEY_FILE=$keyfile
    MATRIX_FORGET_KEY=1
    export GOALCUP_TEST_APPROVAL_KEY_FILE MATRIX_FORGET_KEY
    inner_home_config "$h"
    # Every connector is chat mode (D57): it runs signed in, on this volume's home.
    [ "$(inner_home_ready "$h")" = yes ] ||
        fail "$h is not signed in on this volume: run scripts/linux-matrix.sh login $h first"
    inner_settle
    export MATRIX_CONFIG_DIR=$CFG
    # Claude Code asks whether it may trust each new workspace, "No, exit" selected: Down, Enter.
    if [ "$h" = claude-code-cli ] && [ -z "${MATRIX_PRE_PROMPT_KEYS+set}" ]; then
        MATRIX_PRE_PROMPT_KEYS='\033[B\r'
        export MATRIX_PRE_PROMPT_KEYS
    fi
    mkdir -p "$TMPDIR" || fail "tmp"
    marker=$TMPDIR/run-start.$$
    : >"$marker"
    log=/logs/matrix-$h${LINUX_MATRIX_CELL:+-$LINUX_MATRIX_CELL}-$(date -u +%Y%m%dT%H%M%SZ).log
    # A repository matrix.sh makes is named after the cell.
    [ -z "${LINUX_MATRIX_CELL:-}" ] || export MATRIX_CELL="$LINUX_MATRIX_CELL"
    sh /src/scripts/matrix.sh "$h" "$LINUX_MATRIX_SERVER" --log "$log" "$@" >"$TMPDIR/matrix.$$.out"
    rc=$?
    row=$(grep "^$h " "$TMPDIR/matrix.$$.out" | tail -n 1)
    if ! inner_model_check "$h" "$marker" >"$TMPDIR/model.$$.out" 2>&1; then
        # A run that broke the owner rule never shows a PASS row: matrix.sh's row says FAIL.
        sed -e "s/^\($h [^ ]* [^ ]*\) PASS \([0-9-]*\)\$/\1 FAIL \2/" \
            -e "s/^\($h [^ ]* [^ ]*\) NOT-VERIFIED \([0-9-]*\)\$/\1 FAIL \2/" "$TMPDIR/matrix.$$.out"
        cat "$TMPDIR/model.$$.out"
        echo "linux-matrix: FAIL owner rule: $(rule_of "$h")"
        exit 1
    fi
    cat "$TMPDIR/matrix.$$.out" "$TMPDIR/model.$$.out"
    echo "linux-matrix: log on the host: ${LINUX_MATRIX_HOST_LOG_DIR:-/logs}/$(basename "$log")"
    echo "linux-matrix: ${row:-no row (matrix.sh exit $rc)}"
    exit "$rc"
}

# The fetch credentials, handed over when matrix.sh asked for them: two lines on standard input
# (the read-only database URL, the R2 token; either may be empty) into the private directory
# matrix.sh made for them, which matrix.sh empties and removes as soon as it has read them.
inner_fetch_creds() {
    [ -d "$FETCH_SECRETS_DIR" ] && [ -O "$FETCH_SECRETS_DIR" ] ||
        fail "fetch credentials: matrix.sh made no private directory for them"
    db="" cf=""
    IFS= read -r db
    IFS= read -r cf
    umask 077
    [ -z "$db" ] || printf '%s' "$db" >"$FETCH_SECRETS_DIR/db-url" || fail "fetch credentials: the database URL"
    [ -z "$cf" ] || printf '%s' "$cf" >"$FETCH_SECRETS_DIR/cf-token" || fail "fetch credentials: the R2 token"
    : >"$FETCH_SECRETS_DIR/ready"
    db="" cf=""
}

case ${1:-} in
inner-*) trap forget_secrets EXIT ;;
esac
case ${1:-} in
inner-fetch-creds) inner_fetch_creds; exit 0 ;;
inner-home-config) inner_home_config "$2"; exit 0 ;;
inner-home-files) inner_home_files "$2"; exit 0 ;;
inner-login) inner_home_config "$2"; inner_login; exit 0 ;;
inner-home-ready) inner_home_ready "$2"; exit 0 ;;
inner-model-check) inner_model_check "$2" "$3"; exit $? ;;
inner-settle) inner_settle; exit 0 ;;
inner-run) shift; inner_run "$@" ;;
esac

# ------------------------------------------------------------------ on this machine

[ $# -ge 1 ] || bad_usage "no command (build, login, github or run)"
action=$1
shift
h=""
extra=""
case $action in
-h | --help) usage ;;
build) ;;
login | github | run)
    [ $# -ge 1 ] || bad_usage "$action needs a connector"
    h=$1
    shift
    if [ $# -gt 0 ]; then
        [ "$action" = run ] && [ "$1" = -- ] || bad_usage "unexpected argument $1 after $action $h"
        shift
    fi
    # matrix.sh's options, checked one by one so they can travel as plain words: what a long run
    # needs, and nothing that would change the server, the log or the mode of this script.
    while [ $# -gt 0 ]; do
        case $1 in
        --test | --keep) extra="$extra $1" ;;
        --duration | --timeout)
            [ $# -ge 2 ] || fail "$1 needs a number of seconds"
            case $2 in '' | *[!0-9]*) fail "$1 needs a number of seconds" ;; esac
            extra="$extra $1 $2"
            shift
            ;;
        *) fail "matrix.sh option $1 is not passed on (only --test, --duration N, --timeout N, --keep)" ;;
        esac
        shift
    done
    case $h in *[!a-z0-9-]* | "") fail "the connector id may hold only a-z, 0-9 and -" ;; esac
    ;;
*) bad_usage "unknown command $action" ;;
esac

cell=${LINUX_MATRIX_CELL:-}
if [ -n "$cell" ]; then
    case $cell in *[!a-z0-9-]* | -*) fail "LINUX_MATRIX_CELL may hold only a-z, 0-9 and - (and not start with -)" ;; esac
    [ "${#cell}" -le 40 ] || fail "LINUX_MATRIX_CELL is at most 40 characters"
fi
prebuilt=${LINUX_MATRIX_PREBUILT:-0}
case $prebuilt in 0 | 1) ;; *) fail "LINUX_MATRIX_PREBUILT is 0 or 1" ;; esac
effort=$(codex_effort) || fail "LINUX_MATRIX_CODEX_EFFORT must be low or minimal"

cargo_home=${CARGO_HOME:-$HOME/.cargo}
case $cargo_home in /*) ;; *) cargo_home=$(pwd)/$cargo_home ;; esac
cd "$(dirname "$0")/.." || exit 1

# How a harness is installed, as a participant installs it, at the connector's version.
recipe_of() { # connector version
    case $1 in
    codex-cli) echo "npm install -g @openai/codex@$2 && codex --version" ;;
    claude-code-cli) echo "npm install -g @anthropic-ai/claude-code@$2 && claude --version" ;;
    # docs/harnesses/amp-cli.md: npm install --prefix <dir> @ampcode/cli@<v>; bin/amp is the
    # native executable (the regular file the daemon records).
    amp-cli) echo "npm install --prefix /opt/amp @ampcode/cli@$2 && ln -s /opt/amp/node_modules/@ampcode/cli/bin/amp /usr/local/bin/amp && amp --version" ;;
    hermes-cli) echo "python3 -m venv /opt/hermes && /opt/hermes/bin/pip install --no-cache-dir hermes-agent==$2 && ln -s /opt/hermes/bin/hermes /usr/local/bin/hermes" ;;
    *) return 1 ;;
    esac
}

# The connector's manifest, version, install command and image name.
image_of() { # connector
    manifest=connectors/$1/harness.toml
    [ -f "$manifest" ] || fail "no connector $manifest"
    version=$(version_of "$manifest")
    [ -n "$version" ] || fail "the first line of $manifest names no version"
    install=$(recipe_of "$1" "$version") || fail "no install recipe for $1 in this script (none measured for Linux)"
    img=goalcup-linux-matrix-$1:$version
}

build_image() { # connector (after image_of)
    echo "linux-matrix: image $img"
    docker build -q -t "$img" - >/dev/null <<DOCKERFILE || fail "image $img"
FROM rust:1.99
RUN apt-get update && apt-get install -y --no-install-recommends \
    nodejs npm python3 python3-venv procps libnss-myhostname zstd postgresql-client gh \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i 's/^hosts:.*/hosts:          files myhostname dns/' /etc/nsswitch.conf \
 && useradd -m -u 1000 dev
RUN $install
ENV RUSTUP_TOOLCHAIN=1.99.0 CARGO_TARGET_DIR=/target CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
DOCKERFILE
}

if [ "$action" = build ]; then
    if [ $# -eq 0 ]; then
        for d in connectors/*/; do
            c=$(basename "$d")
            [ -f "$d/harness.toml" ] && recipe_of "$c" 0 >/dev/null && set -- "$@" "$c"
        done
    fi
    [ $# -gt 0 ] || fail "no connector to build"
    for c in "$@"; do
        case $c in *[!a-z0-9-]* | "") fail "the connector id may hold only a-z, 0-9 and -" ;; esac
        image_of "$c"
    done
    built="$*"
else
    image_of "$h"

    server=${LINUX_MATRIX_SERVER:-$STAGING}
    case $server in
    https://*) ;;
    *) fail "LINUX_MATRIX_SERVER must be https://" ;;
    esac
    # Host names are case-insensitive: compare the lowercased address.
    case $(printf '%s' "$server" | tr '[:upper:]' '[:lower:]') in
    *ingest.agentcommunity.org*) fail "production is never a matrix target" ;;
    esac
    member=${GOALCUP_TEST_MEMBER_ID:-$SYNTHETIC_MEMBER}
    case $member in *[!A-Za-z0-9_-]* | "") fail "GOALCUP_TEST_MEMBER_ID may hold only letters, digits, _ -" ;; esac
fi

# Secrets: files on this machine only, never mounted into the container (the harness runs there,
# as the same user, and could read a mounted file): each reaches the one `docker exec` that needs it
# on its standard input. A key in the environment is refused, as matrix.sh does.
[ -z "${GOALCUP_TEST_APPROVAL_KEY:-}" ] ||
    fail "GOALCUP_TEST_APPROVAL_KEY in the environment is refused: give the file as GOALCUP_TEST_APPROVAL_KEY_FILE"
keyfile=${GOALCUP_TEST_APPROVAL_KEY_FILE:-}
unset GOALCUP_TEST_APPROVAL_KEY_FILE
dbfile=${MATRIX_DATABASE_URL_FILE:-}
cftokenfile=${MATRIX_CLOUDFLARE_TOKEN_FILE:-}
unset MATRIX_DATABASE_URL_FILE MATRIX_CLOUDFLARE_TOKEN_FILE
# The fetch credentials as values never enter the container (a -e value is on docker's command
# line): they are files. Said once, so a long cell is not silently left unverifiable.
for v in DATABASE_URL CLOUDFLARE_API_TOKEN; do
    eval "isset=\${$v:+set}"
    [ -z "$isset" ] || [ "$action" = build ] ||
        echo "linux-matrix: $v in the environment is not passed into the container; give it as a file (MATRIX_DATABASE_URL_FILE, MATRIX_CLOUDFLARE_TOKEN_FILE)"
    unset "$v"
done
mounted_file() { # path what
    case $1 in /*) ;; *) fail "$2 is given as an absolute path" ;; esac
    case $1 in *:* | *,*) fail "the path of $2 may not hold : or ," ;; esac
    [ -f "$1" ] && [ -r "$1" ] || fail "$1 is not a regular file this user can read"
}
set --
if [ "$action" = login ] || [ "$action" = run ]; then
    [ -n "$keyfile" ] || fail "GOALCUP_TEST_APPROVAL_KEY_FILE is needed (the login and the run are approved with it)"
    mounted_file "$keyfile" "a secret file"
    set --
    promptfile=${MATRIX_PROMPT_FILE:-}
    if [ -n "$promptfile" ]; then
        [ -z "${MATRIX_PROMPT:-}" ] || fail "set MATRIX_PROMPT or MATRIX_PROMPT_FILE, not both"
        mounted_file "$promptfile" "MATRIX_PROMPT_FILE"
        set -- "$@" -v "$promptfile:$PROMPT_IN:ro"
    fi
    for v in $PASS_ENV; do
        eval "isset=\${$v+set}"
        [ -z "$isset" ] || eval "set -- \"\$@\" -e \"$v=\${$v}\""
    done
    # A long run's server copy (matrix.sh, fetch-run.sh): the index and the bucket mirror, or the
    # read-only database URL and R2 token, each a file mounted read-only.
    fetch_index=${MATRIX_FETCH_INDEX:-}
    if [ -n "$fetch_index" ]; then
        mounted_file "$fetch_index" "MATRIX_FETCH_INDEX"
        set -- "$@" -v "$fetch_index:$FETCH_INDEX_IN:ro" -e "MATRIX_FETCH_INDEX=$FETCH_INDEX_IN"
    fi
    fetch_store=${MATRIX_FETCH_STORE:-}
    if [ -n "$fetch_store" ]; then
        case $fetch_store in /*) ;; *) fail "MATRIX_FETCH_STORE is given as an absolute path" ;; esac
        case $fetch_store in *:* | *,*) fail "the path of MATRIX_FETCH_STORE may not hold : or ," ;; esac
        [ -d "$fetch_store" ] && [ -r "$fetch_store" ] || fail "$fetch_store is not a directory this user can read"
        set -- "$@" -v "$fetch_store:$FETCH_STORE_IN:ro" -e "MATRIX_FETCH_STORE=$FETCH_STORE_IN"
    fi
    # The database URL and the R2 token are checked here and handed over later, only when matrix.sh
    # in the container asks for them at the end of a long run (deliver_fetch_creds); not mounted.
    [ -z "$dbfile" ] || mounted_file "$dbfile" "MATRIX_DATABASE_URL_FILE"
    [ -z "$cftokenfile" ] || mounted_file "$cftokenfile" "MATRIX_CLOUDFLARE_TOKEN_FILE"
    # The account id is no secret; the bucket and the fetch's time are settings.
    for v in CLOUDFLARE_ACCOUNT_ID MATRIX_FETCH_BUCKET MATRIX_FETCH_TIMEOUT; do
        eval "val=\${$v:-}"
        [ -z "$val" ] || set -- "$@" -e "$v=$val"
    done
fi

registry=$cargo_home/registry
[ -d "$registry" ] || fail "registry (run cargo fetch --locked first)"
logdir=${LINUX_MATRIX_LOG_DIR:-${TMPDIR:-/tmp}/goalcup-linux-matrix}
mkdir -p "$logdir" || fail "log directory $logdir"
logdir=$(cd "$logdir" && pwd -P)

checkout=$(printf %s "$PWD" | cksum | cut -d' ' -f1)
target=goalcup-linux-matrix-target-$checkout
if [ "$action" = build ]; then
    name=goalcup-linux-matrix-build-$$
    label=goalcup-linux-matrix-build=$checkout
elif [ -n "$cell" ]; then
    # A fixed name an orchestrator can act on, and a label and volume of the cell's own.
    name=goalcup-linux-matrix-$h-$cell
    label=goalcup-linux-matrix=$h/$cell
    volume=goalcup-linux-matrix-$h-$cell
else
    name=goalcup-linux-matrix-$h-$$
    # One label per connector, as the volume is one per connector: every checkout's run of it uses it.
    label=goalcup-linux-matrix=$h
    volume=goalcup-linux-matrix-$h
fi
lock=${GOALCUP_HEAVY_LOCK:-/tmp/goalcup-heavy.lock}
locked=0
# Only a container this run made is removed: a fixed cell name may be another run's.
created=0
unlock() {
    [ "$locked" = 0 ] || rmdir "$lock" 2>/dev/null
    locked=0
}
trap '[ "$created" = 0 ] || docker rm -f "$name" >/dev/null 2>&1; unlock' EXIT
# The run's exec job (matrix.sh in the container), once it is running.
run_job=""
# On a signal during a run, matrix.sh in the container is stopped first so that its cleanup can
# seal the test run (removing the container at once would leave it open for 168 hours); at most
# about 150 s, then the container is removed as always.
on_signal() {
    trap '' INT HUP TERM
    if [ "$created" = 1 ] && [ -n "$run_job" ]; then
        echo "linux-matrix: stopping matrix.sh in $name so that it can seal its test run (at most 150s)"
        docker exec -u dev "$name" pkill -TERM -f scripts/matrix.sh >/dev/null 2>&1
        waited=0
        while [ "$waited" -lt 150 ]; do
            if ! kill -0 "$run_job" 2>/dev/null &&
                ! docker exec -u dev "$name" pgrep -f scripts/matrix.sh >/dev/null 2>&1; then
                break
            fi
            sleep 2
            waited=$((waited + 2))
        done
    fi
    exit 130
}
trap on_signal INT HUP TERM
# The lock directory names no owner, so a caller that holds it says so (GOALCUP_HEAVY_LOCK_HELD=1)
# and is refused at once; anyone else waits a bounded time, never for ever.
take_lock() {
    if ! mkdir "$lock" 2>/dev/null; then
        [ "${GOALCUP_HEAVY_LOCK_HELD:-0}" != 1 ] ||
            fail "lock: you hold the heavy-run lock $lock yourself; this script takes it itself, so run it outside the lock"
        wait_max=${LINUX_MATRIX_LOCK_WAIT:-1800}
        case $wait_max in '' | *[!0-9]*) fail "LINUX_MATRIX_LOCK_WAIT is a number of seconds" ;; esac
        echo "linux-matrix: waiting for the heavy-run lock $lock (at most ${wait_max}s)"
        waited=0
        until mkdir "$lock" 2>/dev/null; do
            [ "$waited" -lt "$wait_max" ] ||
                fail "lock: the heavy-run lock $lock is still held after ${wait_max}s. If you hold it yourself, do not wrap this script in it"
            sleep 5
            waited=$((waited + 5))
        done
    fi
    locked=1
}

# step NAME [docker exec options] -- COMMAND...: as dev, in the background plus `wait`, so a
# signal runs the trap now.
step() {
    s=$1
    shift
    echo "linux-matrix: $s"
    docker exec -u dev "$@" &
    wait $! || fail "$s"
}

cpus=$(docker info --format '{{.NCPU}}' 2>/dev/null)
case $cpus in '' | *[!0-9]* | 0) cpus=6 ;; esac
[ "$cpus" -le 6 ] || cpus=6

# ---- build: the images and the binaries, once, under the lock
if [ "$action" = build ]; then
    take_lock
    for c in $built; do
        image_of "$c"
        build_image
    done
    docker run -d --name "$name" --label "$label" --init --cpus="$cpus" \
        -v "$PWD":/src:ro -v "$registry":/usr/local/cargo/registry -v "$target":/target \
        "$img" sleep infinity >/dev/null || fail "container"
    created=1
    docker exec -u 0 "$name" chown dev /target || fail "target volume"
    step "build" -w /src "$name" cargo build --offline --locked --workspace
    echo "linux-matrix: built (images for $built; binaries in volume $target)"
    exit 0
fi

# ---- login or run
if [ "$prebuilt" = 1 ]; then
    # Nothing is built: the image and the binaries are build's, and the target is read-only.
    docker image inspect "$img" >/dev/null 2>&1 ||
        fail "image $img is missing: run scripts/linux-matrix.sh build $h first"
    target_mount="$target:/target:ro"
else
    take_lock
    build_image
    target_mount="$target:/target"
fi

# A running container of this connector (or cell) is another run's (maybe the owner's sign-in,
# which runs without the lock): refuse, never remove it. A stopped one is left over: remove it.
busy=$(docker ps -q --filter "label=$label")
[ -z "$busy" ] || fail "another linux-matrix container of $h${cell:+ (cell $cell)} is running ($(docker ps --filter "label=$label" --format '{{.Names}}' | tr '\n' ' ')). Wait for it; if it is stale, docker rm -f it"
old=$(docker ps -aq --filter "label=$label")
[ -z "$old" ] || docker rm -f $old >/dev/null 2>&1
# --init: something must reap orphans, as on a real host. No --privileged: nothing is mounted
# inside; the volume is the Docker host's own ext4.
docker run -d --name "$name" --label "$label" --init --cpus="$cpus" \
    -v "$PWD":/src:ro -v "$registry":/usr/local/cargo/registry -v "$target_mount" \
    -v "$volume":$VOL -v "$logdir":/logs "$@" \
    -e HOME=$VOL/home -e TMPDIR=$VOL/tmp -e GOALCUP_CONFIG_DIR=$CFG \
    -e GOALCUP_CONNECTORS_DIR=/src/connectors -e LINUX_MATRIX_SERVER="$server" \
    -e MATRIX_BIN_DIR=$BIN -e LINUX_MATRIX_CODEX_EFFORT="$effort" \
    -e LINUX_MATRIX_CELL="$cell" \
    -e GOALCUP_TEST_MEMBER_ID="$member" -e LINUX_MATRIX_HOST_LOG_DIR="$logdir" \
    -e PATH=$BIN:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    -w $VOL "$img" sleep infinity >/dev/null || fail "container"
created=1
owned="$VOL $VOL/home $VOL/tmp $VOL/work"
[ "$prebuilt" = 1 ] || owned="/target $owned"
docker exec -u 0 "$name" sh -c "mkdir -p $VOL/home $VOL/tmp $VOL/work && chown dev $owned" ||
    fail "volume"

if [ "$prebuilt" = 1 ]; then
    docker exec -u dev "$name" sh -c 'test -x /target/debug/goalcup && test -x /target/debug/goalcup-daemon && test -x /target/debug/goalcup-verify' ||
        fail "no prebuilt binaries in volume $target: run scripts/linux-matrix.sh build first"
    echo "linux-matrix: prebuilt binaries from volume $target (read-only)"
else
    step "build" -w /src "$name" cargo build --offline --locked --workspace
fi
# Installed as a participant installs them (packaging/install.sh: ~/.local/bin), so the harness and
# its login shells find goalcup where a real install puts it, not in the build's /target/debug.
step "install" "$name" sh -c "mkdir -p $BIN && install -m 0755 /target/debug/goalcup /target/debug/goalcup-daemon /target/debug/goalcup-verify $BIN/"

# The fetch credentials into the container for the one moment matrix.sh needs them: when it has
# made the request file, once, as their own `docker exec`; matrix.sh reads them and removes them at
# once. A request nobody asked for (the harness could write the file too) is the weak point of the
# handover: it reaches only a directory matrix.sh made, and only ever once per run.
deliver_fetch_creds() {
    [ -e "$logdir/$fetch_request" ] || return 0
    rm -f "$logdir/$fetch_request"
    [ "$fetch_given" = 0 ] || return 0
    fetch_given=1
    echo "linux-matrix: matrix.sh asks for the fetch credentials: handing them over for that step"
    {
        [ -z "$dbfile" ] || tr -d '\n' <"$dbfile"
        echo
        [ -z "$cftokenfile" ] || tr -d '\n' <"$cftokenfile"
        echo
    } | docker exec -i -u dev "$name" sh /src/scripts/linux-matrix.sh inner-fetch-creds ||
        echo "linux-matrix: the fetch credentials could not be handed over (the long run will say NOT-VERIFIED)"
}

if [ "$action" = run ]; then
    if [ -n "$extra" ]; then
        echo "linux-matrix: run (matrix.sh options:$extra)"
    else
        echo "linux-matrix: run (about 12 minutes; the practice run ends at the server's deadline)"
    fi
    set --
    [ -z "${MATRIX_PROMPT_FILE:-}" ] || set -- "$@" -e MATRIX_PROMPT_FILE=$PROMPT_IN
    # matrix.sh asks for the fetch credentials only when a long run needs them, by making this file
    # in the log directory; they are named as wanted only when their files were given.
    fetch_request=fetch-request-$name
    given=""
    [ -z "$dbfile" ] || given="db"
    [ -z "$cftokenfile" ] || given="${given:+$given }cf"
    if [ -n "$given" ]; then
        rm -f "$logdir/$fetch_request"
        set -- "$@" -e "MATRIX_FETCH_REQUEST=/logs/$fetch_request" -e "MATRIX_FETCH_WILL_GIVE=$given"
    fi
    # The approval key is this exec's standard input. $extra holds checked words only (options and
    # digits), so it splits as meant.
    docker exec -i -u dev "$@" "$name" sh /src/scripts/linux-matrix.sh inner-run "$h" $extra <"$keyfile" &
    run_job=$!
    fetch_given=0
    while kill -0 "$run_job" 2>/dev/null; do
        [ -z "$given" ] || deliver_fetch_creds
        sleep 1
    done
    wait "$run_job"
    exit $?
fi

if [ "$action" = github ]; then
    # The container's own GitHub login, on the volume's HOME: never this machine's.
    if who=$(docker exec -u dev "$name" gh api user --jq .login 2>/dev/null) && [ -n "$who" ]; then
        echo "linux-matrix: signed in to GitHub as $who (volume $volume)"
        exit 0
    fi
    unlock
    if ! [ -t 0 ] || ! [ -t 1 ]; then
        echo "linux-matrix: OWNER STEP: in a terminal, from $PWD, run:"
        echo "  ${cell:+LINUX_MATRIX_CELL=$cell }$([ "$prebuilt" = 1 ] && echo "LINUX_MATRIX_PREBUILT=1 ")sh scripts/linux-matrix.sh github $h"
        exit 3
    fi
    echo "linux-matrix: the tester's step. gh asks how to sign in: choose GitHub.com, HTTPS, Yes to"
    echo "\"Authenticate Git with your GitHub credentials\", and login with a web browser (open the address"
    echo "it prints on this machine and enter the code). The login stays on volume $volume."
    docker exec -it -u dev -e TERM=xterm-256color "$name" gh auth login --hostname github.com --git-protocol https ||
        fail "gh auth login in the container"
    who=$(docker exec -u dev "$name" gh api user --jq .login 2>/dev/null)
    [ -n "$who" ] || fail "no GitHub login in the container after gh auth login"
    echo "linux-matrix: signed in to GitHub as $who (volume $volume)"
    exit 0
fi

loginlog=$logdir/login-$h${cell:+-$cell}-$(date -u +%Y%m%dT%H%M%SZ).log
# Where the harness keeps its login: file names only, on the terminal and in the login log.
record_home_files() {
    docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-home-files "$h" | tee -a "$loginlog"
}

# The login gets the key on its standard input for this step only.
echo "linux-matrix: login"
docker exec -i -u dev "$name" sh /src/scripts/linux-matrix.sh inner-login "$h" <"$keyfile" &
wait $! || fail "login"
if [ "$(docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-home-ready "$h")" = yes ]; then
    step "settle" "$name" sh /src/scripts/linux-matrix.sh inner-settle
    record_home_files
    echo "linux-matrix: signed in ($h on volume $volume; setup recorded it before)"
    exit 0
fi
unlock
if ! [ -t 0 ] || ! [ -t 1 ]; then
    echo "linux-matrix: OWNER STEP: in a terminal, from $PWD, run:"
    echo "  GOALCUP_TEST_APPROVAL_KEY_FILE=$keyfile${cell:+ LINUX_MATRIX_CELL=$cell}$([ "$prebuilt" = 1 ] && echo " LINUX_MATRIX_PREBUILT=1") sh scripts/linux-matrix.sh login $h"
    exit 3
fi
case $h in
claude-code-cli)
    cat <<EOF
linux-matrix: the owner's step. Setup lists its steps, then says "Press Enter to open $h, or
Ctrl-C to stop." Press Enter. Claude Code then opens inside the Linux container, in the tracker
home its runs use (CLAUDE_CONFIG_DIR). Nothing is recorded.
  1. Pick any theme. At the login choice, take the Claude account with your subscription.
  2. It prints an address: open it in your Mac's browser, sign in, and paste the code it shows
     back into the terminal. (No browser opens from the container.)
  3. If it asks whether to trust the folder, choose Yes. Keep the model it has (haiku, set by
     this script). Type no task. Type /exit and press Enter.
Then goalcup setup lists its four steps; no GitHub step: a test run names its repository at
its start.
Where Claude Code keeps its login on Linux is not measured: the file names are listed after.
EOF
    ;;
codex-cli)
    cat <<EOF
linux-matrix: the owner's step. Setup lists its steps, then says "Press Enter to open $h, or
Ctrl-C to stop." Press Enter. $h then opens inside the Linux container, in the tracker home
its runs use. Nothing is recorded.
  1. On the sign-in screen choose "Sign in with Device Code". (The browser sign-in cannot reach
     the container.)
  2. Open the address it shows in your Mac's browser, sign in, and enter the code it shows.
  3. If it offers another model, keep $LUNA. Type no task. Type /exit and press Enter.
Then goalcup setup lists its four steps; no GitHub step: a test run names its repository at
its start.
EOF
    ;;
*)
    cat <<EOF
linux-matrix: the owner's step. Setup lists its steps, then says "Press Enter to open $h, or
Ctrl-C to stop." Press Enter. $h then opens inside the Linux container, in the tracker home
its runs use. Nothing is recorded. Sign in the way it offers (open any address it prints in your
Mac's browser; no browser opens from the container). Type no task, then exit it the way it says.
Then goalcup setup lists its four steps; no GitHub step: a test run names its repository at
its start.
EOF
    ;;
esac
docker exec -it -u dev -e TERM=xterm-256color "$name" sh -c "cd $VOL/home && goalcup setup --harness $h --work-root $VOL/work"
[ "$(docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-home-ready "$h")" = yes ] ||
    fail "setup did not record the sign-in ($h must exit with /exit, status 0); run login again"
docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-settle || fail "settle"
record_home_files
echo "linux-matrix: signed in ($h on volume $volume)"
