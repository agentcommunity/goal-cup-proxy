# GitHub Copilot CLI capture research — v1.0.83 documentation/source preflight

Status: documentation and public-release research only, 2026-09-10. No Copilot
executable, account, model, hook, keychain, configuration, transcript, fixture or
A1-A11 test was used. The planned measurement platform is macOS and remains
unqualified.

## Answer first

GitHub Copilot CLI has three documented local evidence candidates:

1. a per-session `events.jsonl` under the configurable Copilot home;
2. repository hooks carrying session, submitted/transformed prompt, tool and
   subagent events; and
3. a process-configured OpenTelemetry JSON-lines file that can include model
   inputs, outputs, system instructions, tool definitions, calls and results.

That is enough to justify a real isolated spike. It is not enough to claim a
connector or tier. GitHub calls the session files a complete record, but does
not document their event schema, append/rewrite behavior, byte preservation or
large-output paths. The public release repository does not contain the runtime
implementation. Current mutable documentation may also be newer than the pinned
stable executable.

Two early controls can end the spike quickly. `/ask` deliberately sends a side
question without adding its prompt or answer to conversation history; the hook
and OTel surfaces must still expose it as human-origin model traffic. Also,
config-file `userPromptSubmitted` hooks cannot block or rewrite a prompt, so an
over-length goal is a recorded violation unless the CLI itself blocks before
model dispatch. If either hidden human traffic or a global credential/config
write cannot be bounded, decline the harness rather than invent an exception.

## Exact product and release boundary

The subject is the standalone, terminal-native **GitHub Copilot CLI**, invoked as
`copilot`, in its local interactive interface ([CLI modes]). It is distinct
from:

- the retired `gh-copilot` extension, whose suggestion/explanation interface was
  replaced by the standalone agentic CLI ([retired extension]);
- `gh copilot` as a GitHub CLI forwarding/install path, which still executes the
  standalone CLI ([forwarder announcement]);
- Copilot Chat/editor sessions and the GitHub Copilot desktop app; and
- the GitHub Copilot SDK. The shared hooks reference explicitly names behavior
  that exists only for SDK programmatic hooks, so SDK behavior is never evidence
  for a config-file CLI hook ([prompt hook payload]).

The research pin is stable release **v1.0.83**, published 2026-09-04 at
15:38:08 UTC. GitHub marks it non-draft and non-prerelease, and its tag resolves
to commit
`be82101e70f0253b57519bebb9cc9d0f6dfb2ed2` ([v1.0.83 release],
[v1.0.83 commit]). Later `v1.0.84-*` releases visible on 2026-09-10 are marked
prerelease and are outside this preflight.

The tag tree contains `.github/`, `LICENSE.md`, `README.md`, `changelog.md` and
`install.sh`, but no runtime implementation ([v1.0.83 tree]). Its top changelog
entry is 1.0.82 even though the release page is 1.0.83 ([tag changelog top]).
The release assets are executables/packages, not auditable implementation source.
Consequently:

- **Release-derived** below means the exact tag/release metadata or behavior
  recorded in that tag's changelog.
- **Documented** means current GitHub documentation read on 2026-09-10; it is a
  candidate to test on v1.0.83, not proof that every item exists in that build.
- **Source-derived runtime** conclusions are unavailable because the relevant
  source is not public.
- **Unmeasured** means no observation from the executable or its stores.

## Candidate leg 1: per-session event files

**Documented.** `COPILOT_HOME` replaces the complete default `~/.copilot`
configuration/state directory. Its `session-state/` subdirectory contains one
subdirectory per session ID, with `events.jsonl` and workspace artifacts such as
plans, checkpoints and tracked files. `session-store.db` is a subset used for
cross-session search and is not the full session record ([configuration
directory], [session files]).

GitHub describes the session files as a complete record of prompts, responses,
tools and modified-file details used for resume ([session data]). That is a
product-level completeness statement, not a published wire contract. No current
documentation defines:

- event variants, IDs, human/synthetic provenance or model-content fields;
- whether every model request includes system instructions and tool definitions;
- whether `events.jsonl` is append-only across normal work, compaction, rewind,
  crash recovery and resume;
- a local retention duration or automatic pruning rule for session files;
- how large tool output files are named or linked; or
- whether a partially written final JSON line is retained, repaired or replaced.

**Release-derived.** The tagged changelog records the move to
`session-state`, migration of legacy history when resumed, exact session-ID
startup/resume, recovery from partial session-log data, very large event
histories, and large tool output written to disk ([session format lineage],
[exact session ID], [partial-log resume], [large event history],
[large-output lineage]). These entries show that relevant mechanisms exist in
the v1.0.83 lineage; they do not define their bytes.

**Capture consequence.** A future tailer must read the selected
`$COPILOT_HOME/session-state/$SESSION_ID/events.jsonl` by inode plus byte offset,
preserve every complete raw line, keep a partial tail uncommitted, and emit a
store-replaced signal on truncation or inode change. At shutdown it must copy the
whole selected session directory read-only and compare the tailed bytes with the
final file. No SQLite row or rendered export can substitute for those bytes.

Partitioning is available in principle because the launch can choose an exact
UUID and hooks expose both `sessionId` and `cwd`. It is still unmeasured. The
capture process must bind the selected UUID to the canonical workspace before
persistence and discard every other session directory, even when another CLI
process uses the same isolated home.

## Candidate leg 2a: repository hooks

### Loading, trust and disabling

**Documented.** Repository hooks live in `.github/hooks/*.json`. Copilot CLI
also reads user hooks under `COPILOT_HOME/hooks/`, inline hooks from Copilot and
cross-tool Claude settings, installed-plugin hooks, and machine policy hooks.
All matching sources are combined. Policy hooks are machine-wide, precede other
hooks, survive `disableAllHooks`, and operate regardless of folder trust
([hook locations]).

The same hooks page says the load order is policy, user, project, plugins but its
immediately following source list places repository files before user files.
Because a `postToolUse` hook may rewrite the result, the final cross-source
mutation order is therefore not safe to infer. The future spike must use an empty
isolated home, reject installed/user/policy hooks, and measure what the tracker
hook sees before and after a controlled second project hook.

Repository hooks load only after the user confirms folder trust, and current
how-to guidance says hook changes load when the CLI starts ([trusted startup],
[hook setup]). A structurally invalid hook file is rejected, while an invalid
item may be dropped without rejecting valid siblings ([hook configuration]).
The repository settings flag `disableAllHooks` can suppress project/user/plugin
hooks; policy hooks remain active ([disable hooks]). The arming gate must
therefore require a fresh, authenticated `sessionStart` post with the exact
workspace and session ID after trust, rather than accepting file presence.

### Useful event payloads

**Documented.** The current hook reference exposes:

- `sessionStart`: `sessionId`, `cwd`, `source` (`startup`, `resume`, or `new`) and
  optional initial prompt;
- `userPromptSubmitted`: `sessionId`, `cwd`, and raw `prompt`;
- `userPromptTransformed`: raw prompt plus the runtime-transformed content sent
  to the model and stored in session history;
- `preToolUse`: tool name and arguments;
- `postToolUse`: tool name/arguments and successful `textResultForLlm`;
- `postToolUseFailure`: tool name/arguments and error;
- `agentStop` and `preCompact`: the main transcript path;
- `subagentStart`: transcript path and agent identity; and
- `subagentStop`: agent ID/type/name, transcript path and full final response
  before large-response spill handling ([hook events],
  [session and prompt payloads], [transformed prompt], [tool payloads],
  [subagent payloads]).

This is unusually strong evidence for human prompts, model-facing transformed
user content, tools and subagent completion. It is not a complete model-request
leg: the prompt hooks do not carry system instructions or tool definitions, and
`postToolUse` is itself mutable. Current docs do not prove these events or their
exact shapes on stable v1.0.83. The exact binary must reject an unknown event at
preflight rather than silently omitting it.

The built-in `general-purpose` agent explicitly emits neither `subagentStart`
nor `subagentStop`; other named YAML agents such as `task` and `explore` do
([subagent payloads]). A future A4 control must invoke `task`, plus one
`general-purpose` control, and compare hooks, session files and OTel instead of
assuming all subagents share a hook contract.

### No silent mutation and no prompt gate

Tracker hooks must be observational: command hooks return no decision object and
must never use prompt hooks, `additionalContext`, `modifiedResult`,
`modifiedResponse`, or stop-hook blocking. Those outputs can add or rewrite
model-visible content ([hook mutation outputs]).

For config-file hooks, `userPromptSubmitted.modifiedPrompt` is discarded, while
`userPromptTransformed` can rewrite but cannot block or handle the turn
([prompt hook payload], [transformed prompt]). Command hook exit code 2 is only a
warning for most events. `preToolUse` is different: command errors/non-zero exits
fail closed, but timeouts always fail open; HTTP `preToolUse` failures also fail
open ([hook failures]). The tracker cannot claim a 4,000-character input gate
from this interface. The A7 spike must determine whether the native CLI blocks
before a `chat` span; otherwise Goal Cup records an over-length violation.

For a loopback HTTP receiver, non-TLS localhost is accepted only when
`COPILOT_HOOK_ALLOW_LOCALHOST=1`; permission-granting hook events require HTTPS
([HTTP hooks]). The smallest spike should therefore reuse the existing local
receiver/tailer/checker pattern and a narrow command-post shim for observational
hooks. A per-run receiver token belongs in the tracker-written hook configuration
or process environment and must never be logged.

## Candidate leg 2b: local OpenTelemetry JSONL

**Documented.** Copilot CLI can write all OTel signals as JSON-lines to
`COPILOT_OTEL_FILE_EXPORTER_PATH`. Setting that path enables OTel; an explicit
`COPILOT_OTEL_EXPORTER_TYPE=file` prevents choosing the OTLP network exporter.
With `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true`, chat spans add
full input messages, output messages, system instructions and tool definitions;
tool spans add call arguments and results ([OTel monitoring]).

Each chat span includes the conversation ID, requested and resolved models,
response ID, turn/interaction IDs and token counts. The span tree includes
subagent `invoke_agent` spans and child model calls. Compaction/truncation events
carry counts and, with content capture, the compaction message; the exact tag
changelog says a compaction summary appears as a `CompactionPart` in subsequent
input messages ([OTel monitoring], [OTel compaction lineage]).

This is the only documented candidate for complete model-input evidence. It
still needs measurement for v1.0.83, byte preservation, JSON encoding, field
limits, sampling, ordering, subagent parentage, write/flush latency and hard-kill
loss. “Full” in the docs does not establish that large attributes are unbounded
or identical to provider request bytes. The file must be treated as raw capture,
not as a derived report.

A future spike must set no OTLP endpoint or headers, place the file only in the
ignored artifact root, and prove from the file itself that exporter type is
local. The OTel file is not a provider proxy: subscription-authenticated model
traffic continues directly between the CLI and GitHub.

## Human versus synthetic content

`userPromptSubmitted` is the strongest documented positive human witness, but it
is not sufficient alone. Several paths create model-visible user-role content
without an ordinary typed prompt:

- a `sessionStart` prompt hook auto-submits text as if the user typed it;
- a notification hook may inject `additionalContext` as a user message;
- `agentStop`/`subagentStop` blocking forces another model turn using the hook
  reason;
- `postToolUse` may append additional context or replace the tool result;
- subagents receive parent-authored tasks; and
- custom instructions are copied into prompts ([prompt hooks],
  [notification injection], [hook mutation outputs], [custom instructions]).

The isolated home and tracker project hook should contain none of those mutation
outputs. Any machine policy hook or unexpected project configuration is a
preflight refusal, not an exclusion guessed after the run.

The more difficult positive control is `/ask` (alias `/btw`). GitHub documents
that this local interactive side question uses the current conversation as
context but stores neither question nor answer in conversation history
([side questions]). Its behavior in `events.jsonl`, submitted/transformed hooks
and OTel is unmeasured. If a typed side question reaches a model without both a
leg-2 human witness and recoverable content, the harness cannot support the
Goal Cup rule.

A connector human rule can be designed only after fixtures distinguish ordinary
TUI prompts, queued prompts, `ask_user` replies, `/ask`, synthetic hook content,
subagent tasks, compaction summaries, tool results and harness notifications.
No prefix or user-role heuristic is acceptable.

## Resume, models, subagents and large output

**Resume.** `--session-id UUID` resumes a matching session and creates a new one
only when the absent value is a valid UUID. `--resume` also accepts IDs/prefixes
or names and can select remote tasks, so the future launch should use only an
explicit UUID with `--session-id` ([session selection]). After a native
process-tree kill, relaunch with the same UUID and all the same pins. Require a
`sessionStart.source=resume`, the same event directory, the same OTel
conversation ID, and a fresh completed turn. An absent name/prefix under
`--resume` is the safer no-new-session failure control.

**Models.** Current docs expose `--model`, session-scoped `/model`, and OTel's
requested/resolved model fields ([model selection], [OTel monitoring]). Start
with a named account-available model rather than `auto`, switch once after a
completed turn, and reconcile every chat span and session event. Availability is
account/policy dependent and must be checked on the installed binary; the
research does not pin a claim about which models the participant can use.

**Subagents.** The hook path names a subagent transcript and stop-time agent ID;
OTel offers hierarchical spans; the session store must supply the durable child
and parent relationship. The tagged changelog also says hook lifecycle spans
from a subagent are recorded on the child and re-emitted on the parent
([subagent hook lineage]). This relationship remains unmeasured, particularly
for `general-purpose` where start/stop hooks are documented absent.

**Large output and compaction.** Generate deterministic sentinel output large
enough to force the documented spill behavior, then compare raw shell output,
all new files under the selected session directory, `textResultForLlm`, OTel
`gen_ai.tool.call.result`, and the next model input. Trigger both manual and
natural/automatic compaction. Preserve pre-compaction events and prove the
subsequent model input contains the actual summary. Rendered TUI previews are
never evidence; tagged release history records preview truncation separately
from disk spills ([large-output lineage]).

## Isolation and subscription authentication

### Documented locations and precedence

`COPILOT_HOME` redirects configuration, session state, logs, installed plugins,
permissions and application state. Cache remains platform-global unless
`COPILOT_CACHE_HOME` is separately set ([configuration directory],
[cache location]). Settings precedence is built-in, managed policy, isolated user,
repository, local repository, environment, then flags, with managed exceptions.
Copilot also reads a shared subset from repository `.claude/settings*.json`
([settings precedence]).

Session sync and remote access are enabled by default in current settings.
`remoteExport=false` opts out of normal syncing, but `remote=true` can still
enable export/steering. The future launch must therefore use both `--no-remote`
and `--no-remote-export`; remote/cloud/attached/editor modes are outside the
spike ([session syncing], [session options]).

Auto-update defaults on, and `COPILOT_AUTO_UPDATE=false` is the documented
process override. Without `COPILOT_HOME`, older XDG state may migrate into the
real home; with it, the isolated directory is authoritative. The future run must
also make the executable path/version immutable for that run and resample
`copilot version` outside the model session before and after
([environment variables], [configuration directory]).

### Credential boundary

Interactive OAuth stores a token in the macOS Keychain under service
`copilot-cli`, falling back to plaintext `config.json` only when the credential
store is unavailable. Credential lookup precedence is
`COPILOT_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`, stored OAuth, then GitHub CLI
fallback ([authentication]). There is no documented isolated keychain selector.

The smallest acceptable candidate is an already-authorized, subscription-backed
token supplied **only** as process-scoped `COPILOT_GITHUB_TOKEN`, with `GH_TOKEN`
and `GITHUB_TOKEN` absent. Do not run `copilot login`, read `gh` configuration,
or configure BYOK/provider endpoints. The token value is never printed, copied,
hashed or placed in hook/OTel resource attributes. Future baseline evidence for
credential-bearing `config.json` is metadata-only: existence, file type, mode,
size and modification time as available. A Keychain baseline records at most
existence of the `copilot-cli` service item, never account labels, token bytes or
a digest. If even existence cannot be measured without exposing personal state,
A10 stays unmeasured and the harness is refused.

Content hashes are reserved for non-secret tracker hook/control files. The
future run must snapshot the real `~/.copilot`, `~/Library/Caches/copilot`, prior
XDG locations, GitHub CLI config and the policy-hook path by metadata only; it
must never open credential contents. Any real-home or keychain mutation is not
covered by an existing Goal Cup app-wide exception.

## Smallest future A1-A11 spike

### Paths and candidate pins

Create four disjoint paths: an ignored pinned executable root `SRC`, an isolated
Copilot state/cache root `COPILOT_SPIKE_HOME`, a fresh tracker-created workspace
`W`, and an ignored raw artifact root `RAW`. `RAW` and `COPILOT_SPIKE_HOME` must
not be inside, adjacent to, or ancestors of `W`. Leave the process's system
`HOME` unchanged.

Install and hash only v1.0.83 into `SRC` in the future authorized spike. Before
launch, set:

```text
COPILOT_HOME=$COPILOT_SPIKE_HOME/state
COPILOT_CACHE_HOME=$COPILOT_SPIKE_HOME/cache
COPILOT_AUTO_UPDATE=false
COPILOT_GITHUB_TOKEN=<operator-supplied secret; never logged>
COPILOT_HOOK_ALLOW_LOCALHOST=1
COPILOT_OTEL_ENABLED=true
COPILOT_OTEL_EXPORTER_TYPE=file
COPILOT_OTEL_FILE_EXPORTER_PATH=$RAW/otel.jsonl
OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true
OTEL_SERVICE_NAME=goalcup-copilot-spike
OTEL_RESOURCE_ATTRIBUTES=goalcup.session_id=$SESSION_ID
```

Unset `GH_TOKEN`, `GITHUB_TOKEN`, all provider/BYOK variables, all OTLP endpoint
and header variables, and proxy variables. No model base URL or proxy is
permitted. Launch the pinned executable from `W` with:

```text
copilot -C "$W" --session-id "$SESSION_ID" --model="$MODEL_A" \
  --no-remote --no-remote-export --no-ask-user --allow-all
```

This is the main autonomous-session pin. `--no-ask-user` disables the
`ask_user` tool ([session options]), so that session cannot supply the A2
`ask_user` control. Run that one control in a separate real-TUI probe. Set
`SESSION_ID` and the OTel resource label to a fresh probe UUID, require fresh
arming, omit only `--no-ask-user` from the CLI flags, and keep every other
isolation/environment pin unchanged. Stop the probe at a completed boundary,
restore the original UUID/resource label and flag, then re-arm before continuing
the main run. Keep the probe evidence separate and never claim it occurred in
the main pinned session.

`MODEL_A` and later `MODEL_B` must be explicit models reported available by that
exact binary/account immediately before the measured session. Do not use `auto`.
Do not use `-p`, ACP, editor/app attach, cloud, remote, delegate, worktree, voice,
experimental scheduling, plugins or external MCP servers.

Write one repository-scoped `.github/hooks/goalcup.json` using the current
version-1 hook format and observational command posts for `sessionStart`,
`userPromptSubmitted`, `userPromptTransformed`, `preToolUse`, `postToolUse`,
`postToolUseFailure`, `agentStop`, `subagentStart`, `subagentStop`, `preCompact`
and `sessionEnd`. Every handler returns empty success. If v1.0.83 rejects any
name, retain the failure and revise the measured set; never silently delete an
event from the acceptance bar. Hash the hook file and every non-secret control
file. Watch `.github/hooks/*.json`, `.github/copilot/settings*.json` and
`.claude/settings*.json` for a disabling or mutating change.

### Ordered run

1. Start the existing stub receiver and raw inode/offset tailer before Copilot.
   Prove receiver liveness. Confirm the OTel path does not exist yet.
2. Launch the real interactive TUI. Accept trust for this isolated workspace.
   Require one authenticated `sessionStart` post with matching UUID, canonical
   `cwd`, and fresh source before presenting the goal. Reject policy/user/plugin
   hooks, remote state, unexpected global config, an unrecognized tracker event,
   or a missing OTel file.
3. Submit a byte-distinct goal that writes a file and runs a command. Compare the
   exact bytes across `userPromptSubmitted`, `userPromptTransformed`, raw session
   events and the first OTel chat input before accepting T0.
4. In the main session, exercise an ordinary `keep going`, a queued `keep going`
   and `/ask SENTINEL`. The side question is an immediate acceptance control:
   its human text and model call must be captured even though main conversation
   history omits it. Then run the separate fresh-UUID, freshly armed TUI probe
   described above without `--no-ask-user`, exercise one `ask_user` response,
   stop it at a completed boundary, and restore the main UUID, OTel resource
   label and flag before returning to the original session.
5. Invoke the named `task` subagent and a `general-purpose` control. Require child
   artifacts, OTel hierarchy and a durable parent link; record the documented
   hook absence for `general-purpose` rather than treating it as silence.
6. Run a controlled successful tool, failing tool, and deterministic large-output
   tool. Preserve raw command output and every new session artifact. Compare
   model-visible hook/OTel content, then trigger manual and automatic compaction.
7. Complete a turn, switch from `MODEL_A` to `MODEL_B`, and run one trivial turn.
   Require requested/resolved model identity on every OTel chat span and a
   reconcilable session event.
8. Submit a 4,001-character goal control. Determine from hook ordering and absence
   or presence of a chat span whether it was blocked before model dispatch. If
   the native CLI does not block, record the Goal Cup violation; do not emulate a
   gate with a mutating hook.
9. Submit the disallowed human message `also add a README`. It must remain
   byte-recoverable and classified as human.
10. While a response is in flight, terminate the exact native process tree.
    Preserve the partial tail and pre-kill OTel bytes. Relaunch with the same UUID
    and every pin, require `source=resume`, then submit `keep going` and wait for
    a new completed turn. Run an absent name/prefix `--resume` failure control
    separately and prove it creates no session.
11. Run a parallel interactive CLI with the same isolated home but a different
    UUID and workspace. Collect its control evidence separately under `RAW`, but
    require the candidate capture partition to persist zero foreign records.
12. Exit cleanly. Read-copy the selected session directory, logs, hook stream and
    OTel JSONL without rewriting them. Compare tail versus final bytes, account
    for partial/replaced files, then score A1-A11. Run one separate `-p`
    comparison only after the TUI result; it cannot qualify the connector.

Raw artifacts stay under ignored spike output. Any proposed fixtures are copies
redacted with the repository tool and reviewed by eye. Receipt-bound/raw bytes
are never rewritten.

## Acceptance bar after measurement

A `full` recommendation is possible only if the exact stable binary proves two
independent, partitionable raw legs; every human channel including `/ask` is
witnessed; OTel or another raw surface reconstructs every model-visible request,
response and tool result; subagent lineage resolves; exact-ID resume and
partial-tail recovery are lossless; hooks remain armed; and A10 finds no
app-wide write.

`elevated_review` may be defensible when human provenance and all ordinary model
traffic are complete but a named, bounded auxiliary gap remains reviewable. It
cannot excuse invisible human input, missing model-visible content, foreign
session persistence, or an unbounded global write.

Refuse when `/ask` or another human channel reaches a model without capture,
repository hooks cannot be armed reliably, OTel is absent/lossy/unbounded in
failure behavior, the session file rewrites defeat faithful tailing, remote
export cannot be disabled, or authentication/configuration touches real user
state. This preflight assigns no support tier.

## Open questions for the real spike

1. What exact variants and link fields occur in v1.0.83 `events.jsonl`, and is the
   file append-only through every tested transition?
2. Does v1.0.83 implement every currently documented hook, especially
   `userPromptTransformed`, with the published payload?
3. Does `/ask` fire submitted/transformed hooks and OTel content despite being
   absent from conversation history?
4. Are OTel message, system, tool and subagent attributes byte-complete and
   flushed before/after hard interruption?
5. What disk artifact holds a large tool result, and which representation is the
   exact `textResultForLlm` used in the next request?
6. Which hook observes the original versus final result when another project
   hook mutates `postToolUse` output?
7. How do parent session ID, subagent ID, transcript path and OTel parent span
   join for `task` and for the hook-less `general-purpose` agent?
8. Does exact-UUID resume append to the same inode and emit a reliable resume
   source after an in-flight kill?
9. Can all session, cache, log, permission, update and credential behavior remain
   inside the two documented override roots without touching Keychain or real
   home paths?

[v1.0.83 release]: https://github.com/github/copilot-cli/releases/tag/v1.0.83
[v1.0.83 tree]: https://github.com/github/copilot-cli/tree/v1.0.83
[v1.0.83 commit]: https://github.com/github/copilot-cli/commit/be82101e70f0253b57519bebb9cc9d0f6dfb2ed2
[tag changelog top]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L1-L9
[CLI modes]: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/about-copilot-cli#running-copilot-cli
[retired extension]: https://docs.github.com/en/copilot/how-tos/use-copilot-for-common-tasks/use-copilot-in-the-cli
[forwarder announcement]: https://github.blog/changelog/2026-01-21-install-and-use-github-copilot-cli-directly-from-the-github-cli/
[configuration directory]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference#directory-overview
[session files]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference#session-state
[session data]: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/chronicle#how-session-data-is-stored
[session format lineage]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L2890-L2891
[exact session ID]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L1134-L1134
[partial-log resume]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L1010-L1010
[large event history]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L143-L143
[large-output lineage]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L2626-L2626
[hook locations]: https://docs.github.com/en/copilot/reference/hooks-reference#hooks-locations
[trusted startup]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L1810-L1810
[hook setup]: https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-hooks#creating-a-repository-level-hook
[hook configuration]: https://docs.github.com/en/copilot/reference/hooks-reference#hook-configuration-format
[disable hooks]: https://docs.github.com/en/copilot/reference/hooks-reference#disable-all-hooks
[hook events]: https://docs.github.com/en/copilot/reference/hooks-reference#hook-events
[session and prompt payloads]: https://docs.github.com/en/copilot/reference/hooks-reference#hook-event-input-payloads
[prompt hook payload]: https://docs.github.com/en/copilot/reference/hooks-reference#userpromptsubmitted--userpromptsubmit
[transformed prompt]: https://docs.github.com/en/copilot/reference/hooks-reference#userprompttransformed
[tool payloads]: https://docs.github.com/en/copilot/reference/hooks-reference#posttooluse--posttooluse
[subagent payloads]: https://docs.github.com/en/copilot/reference/hooks-reference#subagentstart
[hook mutation outputs]: https://docs.github.com/en/copilot/reference/hooks-reference#posttooluse-output
[hook failures]: https://docs.github.com/en/copilot/reference/hooks-reference#exit-codes-for-command-hooks
[HTTP hooks]: https://docs.github.com/en/copilot/reference/hooks-reference#http-hooks
[prompt hooks]: https://docs.github.com/en/copilot/reference/hooks-reference#prompt-hooks
[notification injection]: https://docs.github.com/en/copilot/reference/hooks-reference#notification-hook
[custom instructions]: https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/overview#custom-instructions
[side questions]: https://docs.github.com/en/copilot/how-tos/copilot-cli/use-copilot-cli/ask-a-side-question
[session selection]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#command-line-options
[model selection]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#supported-models
[OTel monitoring]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#opentelemetry-monitoring
[OTel compaction lineage]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L690-L690
[subagent hook lineage]: https://github.com/github/copilot-cli/blob/v1.0.83/changelog.md#L23-L23
[cache location]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference#changing-the-location-of-the-configuration-directory
[settings precedence]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference#configuration-file-settings
[session syncing]: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/chronicle#session-syncing
[session options]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#command-line-options
[environment variables]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#environment-variables
[authentication]: https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli#about-authentication
