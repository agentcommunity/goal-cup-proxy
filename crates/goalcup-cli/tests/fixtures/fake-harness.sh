#!/bin/sh
# A harness that is not one: it does what Claude Code CLI 2.1.259 does to the tracker's
# seams, and nothing else it does.
#
# It stands in for `claude` in `connectors/claude-code-cli/manifest.toml`'s `[launch]` and
# `[resume]` recipes, so `goalcup start` and `goalcup resume` can be driven end to end on a
# machine that has no harness installed and without a TUI taking the test runner's terminal.
#
# WHAT IT IS GIVEN, and where each thing comes from:
#
#   argv           the SHIPPED recipe's, unchanged — only `[launch] program` is rewritten.
#                  `--settings <path>` (the artifact-root file the daemon wrote) and either
#                  `--session-id <id>` (launch) or `--resume <id>` (resume).
#   cwd            the workspace. `launch::spawn` does `current_dir(plan.cwd)`.
#   $CLAUDE_CONFIG_DIR  the per-run harness home, pinned by `[launch.env]` from
#                  `{harness_home}`. The transcript goes under it, where the daemon's tailer
#                  looks for it.
#
# WHAT IT LEAVES BEHIND:
#
#   hello.py                              in the workspace — the agent's work, so the seal's
#                                         scan has a file to find that the tracker did not
#                                         write.
#   $CLAUDE_CONFIG_DIR/projects/<slug>/<session>.jsonl
#                                         leg 1, with the fields
#                                         `goalcup-verifier/src/parsers/claude_code.rs`
#                                         actually reads and no others invented for it.
#
# No hook posts: the hook leg is cut (m2/wire deletion pass).
#
# It is deliberately NOT a verification fixture. Nothing here proves a trace: the daemon is
# what turns these bytes into one, and M1c Task 16 is what runs this script against the real
# daemon's strict tailer and the real parser. A fixture that satisfied only the in-process
# mock is not done.

set -u

settings=""
session=""
# Which flag named the session is the one thing a launch and a resume differ by here. Spike 4
# A5 is why it matters: a resume is the case the parser counts separately.
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

[ -n "$settings" ] || { echo "fake-harness: no --settings was passed" >&2; exit 64; }
[ -n "$session" ] || { echo "fake-harness: no session id was passed" >&2; exit 64; }
[ -r "$settings" ] || { echo "fake-harness: $settings is not readable" >&2; exit 66; }
[ -n "${CLAUDE_CONFIG_DIR-}" ] || { echo "fake-harness: CLAUDE_CONFIG_DIR is unset" >&2; exit 78; }

workspace=$(pwd)
# `goalcup_daemon::paths::claude_slug`: every character that is not ASCII alphanumeric
# becomes a hyphen. Spelled here the way the daemon spells it, because the tailer looks for
# the transcript under exactly this name.
#
# The two spellings agree only while the path is ASCII, and that is by fixture design rather
# than by accident: `tr` maps a BYTE and `claude_slug` maps a CHARACTER, so one multi-byte
# character would become one hyphen there and two or more here. The workspace this runs in is
# a `tempfile::tempdir()` under the test runner's own temporary directory; a fixture is not
# where a participant's path is handled, and `goalcup_daemon::paths` is where that case
# belongs if it is ever real.
slug=$(printf '%s' "$workspace" | LC_ALL=C tr -c 'A-Za-z0-9' '-')
stamp=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
prompt='write hello.py'
prompt_id="prompt-$session"

# Leg 1: the transcript, where the tailer looks for it.
project="$CLAUDE_CONFIG_DIR/projects/$slug"
mkdir -p "$project"
transcript="$project/$session.jsonl"
{
    # Claude Code opens a transcript with bookkeeping that carries the session id and no
    # timestamp at all; `SessionClaim` sorts an untimed record before every timed one, so
    # this is the session's own first record and the parser is told so honestly.
    printf '%s\n' "{\"type\":\"mode\",\"sessionId\":\"$session\",\"mode\":\"default\"}"
    printf '%s\n' "{\"type\":\"user\",\"sessionId\":\"$session\",\"cwd\":\"$workspace\",\"timestamp\":\"$stamp\",\"version\":\"2.1.259\",\"promptId\":\"$prompt_id\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"isMeta\":false,\"isSidechain\":false,\"isCompactSummary\":false,\"promptSource\":\"typed\",\"origin\":{\"kind\":\"human\"},\"message\":{\"role\":\"user\",\"content\":\"$prompt\"}}"
    printf '%s\n' "{\"type\":\"assistant\",\"sessionId\":\"$session\",\"cwd\":\"$workspace\",\"timestamp\":\"$stamp\",\"version\":\"2.1.259\",\"requestId\":\"req-$session\",\"isSidechain\":false,\"message\":{\"id\":\"msg-$session\",\"role\":\"assistant\",\"model\":\"claude-fable-5-1\",\"content\":[{\"type\":\"text\",\"text\":\"Writing hello.py.\"}],\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}}"
} >>"$transcript"

# The agent's work, in the workspace, which is this process's cwd.
printf '%s\n' 'print("hello from a fake harness")' >hello.py

exit 0
