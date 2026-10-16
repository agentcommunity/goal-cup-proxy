#!/bin/sh
# scripts/linux-matrix.sh login <connector>
# scripts/linux-matrix.sh run <connector>
#
# The matrix (scripts/matrix.sh) for one real harness on Linux, in Docker, against staging.
# docs/OPERATING.md, "The matrix", says what a row proves.
#
# The container builds the Linux binaries from this checkout (as linux-check.sh does) and has the
# harness installed at the version the connector's first line names: codex-cli from npm,
# hermes-cli from PyPI. The goalcup configuration directory, the harness's tracker home in it, the
# work folder and HOME live on a named Docker volume of the connector's own,
# goalcup-linux-matrix-<connector>, so a harness sign-in done once survives every later run.
# `docker volume rm goalcup-linux-matrix-<connector>` forgets it.
#
# login   `goalcup login` against the server (approved with the test key, unattended), then
#         `goalcup prep --harness <connector>` in this terminal (`docker exec -it`): the harness
#         opens in its tracker home and the owner signs in there. Without a terminal, everything
#         up to that point is done and the command to run in one is printed (exit 3). Once prep
#         has recorded the sign-in (`home_ready`), login does not open the harness again.
#         Only for a chat-mode connector.
# run     scripts/matrix.sh <connector> <server> in the container, with MATRIX_CONFIG_DIR on the
#         volume (chat-mode connectors). Prints the matrix row last, and exits with matrix.sh's
#         status.
#
# Owner rules kept here: Codex's tracker home gets `model = "gpt-5.6-luna"` and
# `model_reasoning_effort = "low"` before every login and run (never gpt-6-astra). After the run,
# every turn of the run's new Codex sessions must say gpt-5.6-luna at effort low; a turn with any
# other model, or no turn found at all, fails the run.
#
# Blocked: a connector given a provider key ([capture.credential], hermes-cli) is refused. A
# keyed harness is never uploaded today, so `goalcup start --practice` refuses it (start.rs,
# docs/RULES.md), and no matrix row can pass. The owner decides whether that changes. The code
# path for it (install recipe, MATRIX_KEY_ENV_FILE) is kept; LINUX_MATRIX_ALLOW_KEYED=1 lets it
# through for that work.
#
# Environment. A secret is only ever a FILE, mounted read-only; its path goes to the one command
# that reads it, and its value is never in an argument, a `-e`, a log or this script's output.
#   GOALCUP_TEST_APPROVAL_KEY_FILE  the server's test approval key (needed by both modes).
#   GOALCUP_TEST_MEMBER_ID          the member approved as
#                                   (default 00000000-0000-4000-8000-0000000c0de1).
#   MATRIX_KEY_ENV_FILE             a keyed harness's provider key file (hermes-cli: a
#                                   DEEPSEEK_API_KEY= line, mode 0600), passed to matrix.sh as
#                                   MATRIX_KEY_ENV_FILE. Blocked, see above.
#   LINUX_MATRIX_SERVER             default https://goalcup-ingest-staging.b-9a5.workers.dev;
#                                   production is refused.
#   LINUX_MATRIX_LOG_DIR            where matrix.sh's log lands on this machine
#                                   (default $TMPDIR/goalcup-linux-matrix).
#   GOALCUP_HEAVY_LOCK              the shared heavy-run lock (default /tmp/goalcup-heavy.lock).
#                                   The script takes it for the build and the run and drops it
#                                   before the interactive sign-in. Never run it inside the lock:
#                                   it would wait for its own caller. A wrapper that holds the
#                                   lock sets GOALCUP_HEAVY_LOCK_HELD=1, and the script refuses at
#                                   once; otherwise it waits at most LINUX_MATRIX_LOCK_WAIT
#                                   seconds (default 1800), then fails.
#
# What the harness can read. It runs as the same user as the script's own steps, so it can read
# the mounted key files (/run/goalcup/*, read-only; staging's test key) and the whole checkout at
# /src, untracked files included. Run it from a clean checkout, never one holding secrets (a
# Worker .dev.vars, for example).
#
# Needs: Docker (no privileged container), `cargo fetch --locked` run on this machine first (the
# container builds --offline from its registry), and network.
# Last line: `linux-matrix: <matrix row>`, `linux-matrix: FAIL <step>`, or for a login,
# `linux-matrix: signed in` or `linux-matrix: OWNER STEP ...`.
set -u

STAGING=https://goalcup-ingest-staging.b-9a5.workers.dev
SYNTHETIC_MEMBER=00000000-0000-4000-8000-0000000c0de1
LUNA=gpt-5.6-luna
# Paths inside the container.
VOL=/vol
CFG=$VOL/cfg
KEY_IN=/run/goalcup/approval-key
KEYENV_IN=/run/goalcup/key.env

fail() {
    echo "linux-matrix: FAIL $*"
    exit 1
}
usage() {
    awk 'NR > 1 && !/^#/ { exit } NR > 1 { print }' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

# The connector's version, from its manifest's first line (`# Codex CLI 0.153.0, ...`).
version_of() { # manifest
    sed -n '1s/^#[^0-9]* \([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p' "$1"
}

# Prints `chat` for a connector whose manifest says `interactive = true` under [capture].
mode_of() { # manifest
    awk '/^\[/ { sec = $0 } sec == "[capture]" && /^interactive *= *true/ { c = 1 } END { print c ? "chat" : "headless" }' "$1"
}

# ------------------------------------------------------------------ inside the container
# These run as `dev` in the container, called by the host half below.

# The tracker home's config for a harness that has a model rule.
inner_home_config() { # connector
    case $1 in
    codex-cli)
        home=$CFG/capture-homes/codex-cli
        mkdir -p "$home" && chmod 700 "$CFG/capture-homes" "$home" || fail "codex home"
        python3 - "$home/config.toml" "$LUNA" <<'PY' || fail "codex config"
import os, re, sys

path, model = sys.argv[1], sys.argv[2]
try:
    lines = open(path).read().splitlines()
except FileNotFoundError:
    lines = []
# Top-level keys come before the first table; drop ours there and put them first.
kept, top = [], True
for line in lines:
    if line.lstrip().startswith("["):
        top = False
    if top and re.match(r"\s*(model|model_reasoning_effort)\s*=", line):
        continue
    kept.append(line)
text = "\n".join([f'model = "{model}"', 'model_reasoning_effort = "low"'] + kept) + "\n"
fd = os.open(path + ".new", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
with os.fdopen(fd, "w") as f:
    f.write(text)
os.replace(path + ".new", path)
PY
        echo "linux-matrix: codex home config: model $LUNA, reasoning effort low"
        ;;
    esac
}

# `goalcup login`, its one device code approved through the test route: the key goes from its
# file straight into curl's standard input as a header, as matrix.sh does it.
inner_login() {
    keyfile=${GOALCUP_TEST_APPROVAL_KEY_FILE:-}
    unset GOALCUP_TEST_APPROVAL_KEY_FILE
    [ -f "$keyfile" ] && [ -r "$keyfile" ] || fail "login: the approval key file is not readable in the container"
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

# What prep recorded: `yes` once the owner has signed in to this connector's tracker home.
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

# After prep: the repository and work folder a practice run needs, as matrix.sh settles them for
# its own configuration. The token is format-valid and fake: a practice clone is anonymous and
# installs no push helper, so it is never sent anywhere. prep's own repository step is left as
# `todo` on purpose; with a repository set it would ask for a real token.
inner_settle() {
    mkdir -p "$VOL/work" || fail "work folder"
    python3 - "$CFG" "$VOL/work" "${MATRIX_REPO_URL:-https://github.com/agentcommunity/goalcup-matrix-empty}" <<'PY' || fail "settle prep.json"
import json, os, sys

cfg, work, repo = sys.argv[1:4]

def write(name, doc):
    path = os.path.join(cfg, name)
    fd = os.open(path + ".new", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    os.replace(path + ".new", path)

prep = json.load(open(os.path.join(cfg, "prep.json")))
prep["repo_url"] = prep.get("repo_url") or repo
prep["work_root"] = prep.get("work_root") or work
write("prep.json", prep)
gh = os.path.join(cfg, "github.json")
try:
    same = json.load(open(gh)).get("repo_url") == prep["repo_url"]
except (OSError, ValueError):
    same = False
if not same:
    write("github.json", {"repo_url": prep["repo_url"], "token": "ghp_matrixfaketoken00000000000000000000"})
PY
}

# Codex writes each turn's model and reasoning effort into its rollout, as a `turn_context` line
# (spikes/codex-app/fixtures). Every turn of the run's new sessions must say gpt-5.6-luna at effort
# low; any other model fails, and so does finding no turn at all: the rule cannot be confirmed.
inner_model_check() { # connector marker
    [ "$1" = codex-cli ] || return 0
    python3 - "$CFG/capture-homes/codex-cli/sessions" "$2" "$LUNA" <<'PY'
import json, os, sys

root, marker, want = sys.argv[1], sys.argv[2], sys.argv[3]
since = os.stat(marker).st_mtime
turns = []
for d, _, files in os.walk(root):
    for f in files:
        path = os.path.join(d, f)
        if not (f.startswith("rollout-") and f.endswith(".jsonl")) or os.stat(path).st_mtime < since:
            continue
        for line in open(path, errors="replace"):
            try:
                o = json.loads(line)
            except ValueError:
                continue
            if isinstance(o, dict) and o.get("type") == "turn_context":
                ctx = o.get("payload") or {}
                turns.append((ctx.get("model"), ctx.get("effort")))
seen = sorted({f"{m} at effort {e}" for m, e in turns})
print("linux-matrix: Codex turns of this run: " + (", ".join(seen) if seen else "none found"))
if not turns:
    print("linux-matrix: the model could not be confirmed (no Codex turn in the run's sessions)")
    sys.exit(1)
sys.exit(0 if all(m == want and e == "low" for m, e in turns) else 1)
PY
}

inner_run() { # connector
    h=$1
    inner_home_config "$h"
    if [ "$(mode_of "/src/connectors/$h/harness.toml")" = chat ]; then
        [ "$(inner_home_ready "$h")" = yes ] ||
            fail "$h is not signed in on this volume: run scripts/linux-matrix.sh login $h first"
        inner_settle
        export MATRIX_CONFIG_DIR=$CFG
    fi
    mkdir -p "$TMPDIR" || fail "tmp"
    marker=$TMPDIR/run-start.$$
    : >"$marker"
    log=/logs/matrix-$h-$(date -u +%Y%m%dT%H%M%SZ).log
    sh /src/scripts/matrix.sh "$h" "$LINUX_MATRIX_SERVER" --log "$log" >"$TMPDIR/matrix.$$.out"
    rc=$?
    cat "$TMPDIR/matrix.$$.out"
    row=$(grep "^$h " "$TMPDIR/matrix.$$.out" | tail -n 1)
    if ! inner_model_check "$h" "$marker"; then
        echo "linux-matrix: FAIL owner rule: every Codex turn must be $LUNA at effort low"
        exit 1
    fi
    echo "linux-matrix: log on the host: ${LINUX_MATRIX_HOST_LOG_DIR:-/logs}/$(basename "$log")"
    echo "linux-matrix: ${row:-no row (matrix.sh exit $rc)}"
    exit "$rc"
}

case ${1:-} in
inner-home-config) inner_home_config "$2"; exit 0 ;;
inner-login) inner_home_config "$2"; inner_login; exit 0 ;;
inner-home-ready) inner_home_ready "$2"; exit 0 ;;
inner-model-check) inner_model_check "$2" "$3"; exit $? ;;
inner-settle) inner_settle; exit 0 ;;
inner-run) inner_run "$2" ;;
esac

# ------------------------------------------------------------------ on this machine

[ $# -eq 2 ] || usage
action=$1
h=$2
case $action in login | run) ;; *) usage ;; esac
case $h in *[!a-z0-9-]* | "") fail "the connector id may hold only a-z, 0-9 and -" ;; esac

cargo_home=${CARGO_HOME:-$HOME/.cargo}
case $cargo_home in /*) ;; *) cargo_home=$(pwd)/$cargo_home ;; esac
cd "$(dirname "$0")/.." || exit 1
manifest=connectors/$h/harness.toml
[ -f "$manifest" ] || fail "no connector $manifest"
version=$(version_of "$manifest")
[ -n "$version" ] || fail "the first line of $manifest names no version"
mode=$(mode_of "$manifest")

# How the harness is installed, as a participant installs it, at the connector's version.
case $h in
codex-cli) install="npm install -g @openai/codex@$version && codex --version" ;;
hermes-cli) install="python3 -m venv /opt/hermes && /opt/hermes/bin/pip install --no-cache-dir hermes-agent==$version && ln -s /opt/hermes/bin/hermes /usr/local/bin/hermes" ;;
*) fail "no install recipe for $h in this script" ;;
esac
# A harness given a provider key is never uploaded, so it cannot make the practice run a row is.
if grep -q '^\[capture\.credential\]' "$manifest" && [ "${LINUX_MATRIX_ALLOW_KEYED:-0}" != 1 ]; then
    fail "blocked: $h is given a provider key, and a keyed harness is never uploaded, so goalcup start refuses --practice (docs/RULES.md); the owner decides"
fi
[ "$action" = run ] || [ "$mode" = chat ] ||
    fail "login: $h is headless (no sign-in to keep); give its key with MATRIX_KEY_ENV_FILE to run"

server=${LINUX_MATRIX_SERVER:-$STAGING}
case $server in
https://*) ;;
*) fail "LINUX_MATRIX_SERVER must be https://" ;;
esac
case $server in *ingest.agentcommunity.org*) fail "production is never a matrix target" ;; esac
member=${GOALCUP_TEST_MEMBER_ID:-$SYNTHETIC_MEMBER}
case $member in *[!A-Za-z0-9_-]* | "") fail "GOALCUP_TEST_MEMBER_ID may hold only letters, digits, _ -" ;; esac

# Secrets: files only, mounted read-only. A key in the environment is refused, as matrix.sh does.
[ -z "${GOALCUP_TEST_APPROVAL_KEY:-}" ] ||
    fail "GOALCUP_TEST_APPROVAL_KEY in the environment is refused: give the file as GOALCUP_TEST_APPROVAL_KEY_FILE"
keyfile=${GOALCUP_TEST_APPROVAL_KEY_FILE:-}
keyenv=${MATRIX_KEY_ENV_FILE:-}
unset GOALCUP_TEST_APPROVAL_KEY_FILE MATRIX_KEY_ENV_FILE
[ -n "$keyfile" ] || fail "GOALCUP_TEST_APPROVAL_KEY_FILE is needed (the login and the run are approved with it)"
secret_file() { # path
    case $1 in /*) ;; *) fail "secret files are given as absolute paths" ;; esac
    case $1 in *:* | *,*) fail "a secret file's path may not hold : or ," ;; esac
    [ -f "$1" ] && [ -r "$1" ] || fail "$1 is not a regular file this user can read"
}
secret_file "$keyfile"
[ -z "$keyenv" ] || secret_file "$keyenv"
set -- -v "$keyfile:$KEY_IN:ro"
[ -z "$keyenv" ] || set -- "$@" -v "$keyenv:$KEYENV_IN:ro"

registry=$cargo_home/registry
[ -d "$registry" ] || fail "registry (run cargo fetch --locked first)"
logdir=${LINUX_MATRIX_LOG_DIR:-${TMPDIR:-/tmp}/goalcup-linux-matrix}
mkdir -p "$logdir" || fail "log directory $logdir"
logdir=$(cd "$logdir" && pwd -P)

checkout=$(printf %s "$PWD" | cksum | cut -d' ' -f1)
img=goalcup-linux-matrix-$h:$version
name=goalcup-linux-matrix-$h-$$
# One label per connector, as the volume is one per connector: every checkout's run of it uses it.
label=goalcup-linux-matrix=$h
volume=goalcup-linux-matrix-$h
target=goalcup-linux-matrix-target-$checkout
lock=${GOALCUP_HEAVY_LOCK:-/tmp/goalcup-heavy.lock}
locked=0
unlock() {
    [ "$locked" = 0 ] || rmdir "$lock" 2>/dev/null
    locked=0
}
trap 'docker rm -f "$name" >/dev/null 2>&1; unlock' EXIT
trap 'exit 130' INT HUP TERM
# The lock directory names no owner, so a caller that holds it says so (GOALCUP_HEAVY_LOCK_HELD=1)
# and is refused at once; anyone else waits a bounded time, never for ever.
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

echo "linux-matrix: image $img"
docker build -q -t "$img" - >/dev/null <<DOCKERFILE || fail "image"
FROM rust:1.99
RUN apt-get update && apt-get install -y --no-install-recommends \
    nodejs npm python3 python3-venv procps libnss-myhostname \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i 's/^hosts:.*/hosts:          files myhostname dns/' /etc/nsswitch.conf \
 && useradd -m -u 1000 dev
RUN $install
ENV RUSTUP_TOOLCHAIN=1.99.0 CARGO_TARGET_DIR=/target CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0
DOCKERFILE

# A running container of this connector is another run's (maybe the owner's sign-in, which runs
# without the lock): refuse, never remove it. A stopped one is left over: remove it.
busy=$(docker ps -q --filter "label=$label")
[ -z "$busy" ] || fail "another linux-matrix container of $h is running ($(docker ps --filter "label=$label" --format '{{.Names}}' | tr '\n' ' ')). Wait for it; if it is stale, docker rm -f it"
old=$(docker ps -aq --filter "label=$label")
[ -z "$old" ] || docker rm -f $old >/dev/null 2>&1
cpus=$(docker info --format '{{.NCPU}}' 2>/dev/null)
case $cpus in '' | *[!0-9]* | 0) cpus=6 ;; esac
[ "$cpus" -le 6 ] || cpus=6
# --init: something must reap orphans, as on a real host. No --privileged: nothing is mounted
# inside; the volume is the Docker host's own ext4.
docker run -d --name "$name" --label "$label" --init --cpus="$cpus" \
    -v "$PWD":/src:ro -v "$registry":/usr/local/cargo/registry -v "$target":/target \
    -v "$volume":$VOL -v "$logdir":/logs "$@" \
    -e HOME=$VOL/home -e TMPDIR=$VOL/tmp -e GOALCUP_CONFIG_DIR=$CFG \
    -e GOALCUP_CONNECTORS_DIR=/src/connectors -e LINUX_MATRIX_SERVER="$server" \
    -e GOALCUP_TEST_MEMBER_ID="$member" -e LINUX_MATRIX_HOST_LOG_DIR="$logdir" \
    -e PATH=/target/debug:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    -w $VOL "$img" sleep infinity >/dev/null || fail "container"
docker exec -u 0 "$name" sh -c "mkdir -p $VOL/home $VOL/tmp $VOL/work && chown dev /target $VOL $VOL/home $VOL/tmp $VOL/work" ||
    fail "volume"

# step NAME [docker exec options] -- COMMAND...: as dev, in the background plus `wait`, so a
# signal runs the trap now.
step() {
    s=$1
    shift
    echo "linux-matrix: $s"
    docker exec -u dev "$@" &
    wait $! || fail "$s"
}
step "build" -w /src "$name" cargo build --offline --locked --workspace

if [ "$action" = run ]; then
    set -- -e GOALCUP_TEST_APPROVAL_KEY_FILE=$KEY_IN
    [ -z "$keyenv" ] || set -- "$@" -e MATRIX_KEY_ENV_FILE=$KEYENV_IN
    echo "linux-matrix: run (about 12 minutes; the practice run ends at the server's deadline)"
    docker exec -u dev "$@" "$name" sh /src/scripts/linux-matrix.sh inner-run "$h" &
    wait $!
    exit $?
fi

step "login" -e GOALCUP_TEST_APPROVAL_KEY_FILE=$KEY_IN "$name" sh /src/scripts/linux-matrix.sh inner-login "$h"
if [ "$(docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-home-ready "$h")" = yes ]; then
    step "settle" "$name" sh /src/scripts/linux-matrix.sh inner-settle
    echo "linux-matrix: signed in ($h on volume $volume; prep recorded it before)"
    exit 0
fi
unlock
if ! [ -t 0 ] || ! [ -t 1 ]; then
    echo "linux-matrix: OWNER STEP: in a terminal, from $PWD, run:"
    echo "  GOALCUP_TEST_APPROVAL_KEY_FILE=$keyfile sh scripts/linux-matrix.sh login $h"
    exit 3
fi
cat <<EOF
linux-matrix: the owner's step. Prep lists its steps, then says "Press Enter to open $h, or
Ctrl-C to stop." Press Enter. $h then opens inside the Linux container, in the tracker home
its runs use. Nothing is recorded.
  1. On the sign-in screen choose "Sign in with Device Code". (The browser sign-in cannot reach
     the container.)
  2. Open the address it shows in your Mac's browser, sign in, and enter the code it shows.
  3. If it offers another model, keep $LUNA. Type no task. Type /exit and press Enter.
Then goalcup prep lists its steps. "[todo] github" and "[todo] practice" are expected here.
EOF
docker exec -it -u dev -e TERM=xterm-256color "$name" sh -c "cd $VOL/home && goalcup prep --harness $h --work-root $VOL/work"
[ "$(docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-home-ready "$h")" = yes ] ||
    fail "prep did not record the sign-in ($h must exit with /exit, status 0); run login again"
docker exec -u dev "$name" sh /src/scripts/linux-matrix.sh inner-settle || fail "settle"
echo "linux-matrix: signed in ($h on volume $volume)"
