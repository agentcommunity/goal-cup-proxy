#!/bin/sh
# A `claude` that is not one, installed on PATH under the name
# `connectors/claude-code-cli/manifest.toml` declares (`detection.binary`, `[launch] program`).
#
# WHY IT IS A PATH SHIM AND NOT A REWRITTEN MANIFEST
#
# `tests/fixtures/fake-harness.sh` beside this file is the other fixture, and it is reached by
# rewriting `[launch] program` in a COPY of the connector directory. That copy is a rulebook
# this build does not pin, so a trace captured under it carries `manifest_not_pinned` — a Flag
# even under `--dev` (`checks/v13_binding.rs::manifest_pin_finding`) — and `overall` can never
# be Pass. M1c Task 16's whole subject is a Pass verdict over a trace the real daemon wrote, so
# this fixture leaves the shipped connector byte for byte alone and changes the one thing left:
# which `claude` the machine resolves.
#
# WHAT IT IS GIVEN, and where each thing comes from:
#
#   --version      the D12 probe (`detection.version_command = "claude --version"`). Answered
#                  first, before anything else is parsed, because the probe passes no other
#                  argument and every check below would refuse it.
#   argv           the SHIPPED recipe's, unchanged: `--settings <path>` (the artifact-root file
#                  the daemon wrote) and `--session-id <id>` (launch) or `--resume <id>`.
#   cwd            the workspace. `launch::spawn` does `current_dir(plan.cwd)`.
#   $CLAUDE_CONFIG_DIR  the per-run harness home, pinned by `[launch.env]` from
#                  `{harness_home}`. The transcript goes under it, where the tailer looks.
#
# WHAT IT LEAVES BEHIND:
#
#   five leg-1 records    the untimed `mode` opener `SessionClaim` sorts first; the goal;
#                         an assistant turn carrying a `tool_use` Write; that call's
#                         `tool_result`; and one `keep going`. The Write pair is not
#                         decoration: `attribution.rs` fails a sealed file no traced write
#                         explains (`unexplained_seal_file`), so a harness that wrote
#                         `hello.py` without one would produce a Fail.
#   hello.py              in the workspace — the agent's work, so the seal has a file to find
#                         that the tracker did not write and the trace has to explain.
#
# No hook posts: the hook leg is cut (m2/wire deletion pass), so the transcript is the only
# leg, and the verdict over it is the transcript-only one.
#
# The record SHAPES are `crates/goalcup-devserver/tests/api_gate.rs`'s. What this fixture adds
# is that a PROCESS the CLI launched writes them.
#
# $GOALCUP_FAKE_CLAUDE_LINGER — THE HARNESS IS STILL RUNNING WHEN THE SEAL ARRIVES
#
# Set to any non-empty value, this fixture does everything above and then STAYS ALIVE rather
# than exiting. That is the shape a real run has: the participant seals while their harness is
# open, `goalcup start` is still blocked in `child.wait()`, and the process ends only because
# `Daemon::seal_entry` terminates it. Left unset, the fixture exits on its own and the seal
# kills a process that had already gone — which proves nothing about the kill, about the
# hand-off, or about `start` ever coming back.
#
# The wait is a `sleep` loop and not a `read`: stdin is `/dev/null` under the CLI's launch, so
# a read would return immediately and the "still running" case would be a lie that passed.
# SIGTERM is deliberately NOT trapped. A shell's default disposition is what a harness that
# does no cleanup does, and that is the case the terminator's grace period is written for.
#
# $GOALCUP_FAKE_CLAUDE_EXIT_GATE — FINITE, BUT ONLY AFTER REGISTRATION
#
# The real resume E2E needs the harness to exit by itself, yet it must not outrun the CLI's
# `/process` hand-off and leave its PID available for reuse before the daemon validates the
# sampled process identity. When this test-only path is set, the fixture waits until the test
# creates that file. The test creates it only after the daemon acknowledgement is reflected in
# the entry cache, so this remains a natural harness exit without relying on a timing sleep.

set -u

# The D12 version probe, first and on its own. `sample_harness_version_within` reads the first
# non-empty line of stdout and takes the first version in it; `tested_version_range` is
# ">=2.1.259", and a number below it is a Fail rather than the Warn an absent binary earns.
for arg in "$@"; do
    if [ "$arg" = "--version" ]; then
        echo "2.1.259 (Claude Code)"
        exit 0
    fi
done

settings=""
session=""
# Which flag named the session is the one thing a launch and a resume differ by here, and it is
# the `source` of the session (spike 4 A5).
start_source="startup"
while [ $# -gt 0 ]; do
    case "$1" in
        --settings)
            settings="${2-}"
            shift 2 || exit 64
            ;;
        --session-id)
            session="${2-}"
            shift 2 || exit 64
            ;;
        --resume)
            session="${2-}"
            start_source="resume"
            shift 2 || exit 64
            ;;
        *) shift ;;
    esac
done

[ -n "$settings" ] || { echo "fake-claude: no --settings was passed" >&2; exit 64; }
[ -n "$session" ] || { echo "fake-claude: no session id was passed" >&2; exit 64; }
[ -r "$settings" ] || { echo "fake-claude: $settings is not readable" >&2; exit 66; }
[ -n "${CLAUDE_CONFIG_DIR-}" ] || { echo "fake-claude: CLAUDE_CONFIG_DIR is unset" >&2; exit 78; }

# A readiness marker for the finite-exit test only. The version probe exits above, so this
# proves the actual launched harness received the gate and reached its stable script body.
if [ -n "${GOALCUP_FAKE_CLAUDE_EXIT_GATE-}" ]; then
    : >"$GOALCUP_FAKE_CLAUDE_EXIT_GATE.ready"
fi

# The workspace as THIS process sees it, which is what a real harness reporting `process.cwd()`
# sees too: physically, with every symlink above it resolved. The daemon holds the path it was
# configured with and derives the project slug from `PathVars::workspace_canonical`, so the two
# agree here only because the daemon canonicalises. M1c Task 16 runs this under a deliberately
# symlinked workspace parent for exactly that reason.
#
# `pwd -P` and not a bare `pwd`: a bare one may answer $PWD, which is whatever spelling the
# parent process exported, and this fixture's whole job here is to report the PHYSICAL path
# the way a harness calling `process.cwd()` does. The launch clears the environment so $PWD
# is absent today and the two agree — which is exactly the accident that would make a
# regression here invisible.
workspace=$(pwd -P)
# `goalcup_daemon::paths::claude_slug`: every character that is not ASCII alphanumeric becomes
# a hyphen. Spelled here the way the daemon spells it, because the tailer looks for the
# transcript under exactly this name.
slug=$(printf '%s' "$workspace" | LC_ALL=C tr -c 'A-Za-z0-9' '-')
stamp=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

# Under `MAX_PROMPT_CHARS`: a launch carries the original goal, while a resume continues the
# same session with only the message V2 permits.
keep_going='keep going'
goal='write hello.py that prints a number'
if [ "$start_source" = "resume" ]; then
    goal="$keep_going"
fi
body='print(1)\n'
# A resumed Claude session keeps its session id, but its prompt, call and record identities
# continue from the earlier launch. `start_source` is the fixture's invocation discriminator:
# without it a resumed batch would look like a replay to the verifier rather than a second
# segment of the same session.
p1="prompt-1-$start_source-$session"
p2="prompt-2-$start_source-$session"
call="toolu-1-$start_source-$session"
project="$CLAUDE_CONFIG_DIR/projects/$slug"
transcript="$project/$session.jsonl"
mkdir -p "$project"

record() { printf '%s\n' "$1" >>"$transcript"; }

# ---- the session opens. Claude Code writes bookkeeping that carries the session id and no
# timestamp at all, and `SessionClaim` sorts an untimed record before every timed one.
record "{\"type\":\"mode\",\"sessionId\":\"$session\",\"mode\":\"default\"}"

# ---- the goal.
record "{\"type\":\"user\",\"sessionId\":\"$session\",\"cwd\":\"$workspace\",\"timestamp\":\"$stamp\",\"version\":\"2.1.259\",\"promptId\":\"$p1\",\"uuid\":\"uuid-$p1\",\"parentUuid\":null,\"permissionMode\":\"auto\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"isMeta\":false,\"isSidechain\":false,\"isCompactSummary\":false,\"promptSource\":\"typed\",\"origin\":{\"kind\":\"human\"},\"message\":{\"role\":\"user\",\"content\":\"$goal\"}}"

# ---- the agent's turn: a Write call, which is what explains the sealed file below.
record "{\"type\":\"assistant\",\"sessionId\":\"$session\",\"cwd\":\"$workspace\",\"timestamp\":\"$stamp\",\"version\":\"2.1.259\",\"requestId\":\"req-1-$start_source-$session\",\"uuid\":\"uuid-a1-$start_source-$session\",\"parentUuid\":\"uuid-$p1\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"isSidechain\":false,\"message\":{\"id\":\"msg-1-$start_source-$session\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-fable-5-1\",\"stop_reason\":\"tool_use\",\"content\":[{\"type\":\"text\",\"text\":\"Writing hello.py.\"},{\"type\":\"tool_use\",\"id\":\"$call\",\"name\":\"Write\",\"input\":{\"file_path\":\"$workspace/hello.py\",\"content\":\"$body\"}}],\"usage\":{\"input_tokens\":1200,\"output_tokens\":340,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}"

# The agent's work, in the workspace, which is this process's cwd.
printf "$body" >hello.py

record "{\"type\":\"user\",\"sessionId\":\"$session\",\"cwd\":\"$workspace\",\"timestamp\":\"$stamp\",\"version\":\"2.1.259\",\"promptId\":\"$p1\",\"uuid\":\"uuid-tr1-$start_source-$session\",\"parentUuid\":\"uuid-a1-$start_source-$session\",\"sourceToolAssistantUUID\":\"uuid-a1-$start_source-$session\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"isSidechain\":false,\"message\":{\"role\":\"user\",\"content\":[{\"tool_use_id\":\"$call\",\"type\":\"tool_result\",\"content\":\"Wrote hello.py\",\"is_error\":false}]}}"

# ---- the one steering message R2 allows.
record "{\"type\":\"user\",\"sessionId\":\"$session\",\"cwd\":\"$workspace\",\"timestamp\":\"$stamp\",\"version\":\"2.1.259\",\"promptId\":\"$p2\",\"uuid\":\"uuid-$p2\",\"parentUuid\":\"uuid-tr1-$session\",\"permissionMode\":\"auto\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"isMeta\":false,\"isSidechain\":false,\"isCompactSummary\":false,\"promptSource\":\"typed\",\"origin\":{\"kind\":\"human\"},\"message\":{\"role\":\"user\",\"content\":\"$keep_going\"}}"

# ---- and, where the test asked for it, the harness is still here when the seal arrives.
# Nothing below this line writes anything: every record is already on disk, so the only difference this makes to the trace is that the process
# the daemon holds a handle for is alive when the terminator reaches for it.
if [ -n "${GOALCUP_FAKE_CLAUDE_LINGER-}" ]; then
    while :; do
        sleep 1
    done
fi

if [ -n "${GOALCUP_FAKE_CLAUDE_EXIT_GATE-}" ]; then
    while [ ! -e "$GOALCUP_FAKE_CLAUDE_EXIT_GATE" ]; do
        sleep 0.05
    done
fi

exit 0
