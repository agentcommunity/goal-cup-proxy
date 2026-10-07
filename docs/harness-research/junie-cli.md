# Junie CLI capture research — 3110.7 documentation/source preflight

Status: documentation and public-release research only, 2026-09-10. No Junie
executable, account, model, hook, keychain, configuration, transcript, fixture or
A1-A11 test was used. The planned measurement platform is macOS and remains
unqualified.

## Answer first

The first-party standalone `junie` CLI has one promising local evidence store:
a per-session `events.jsonl` beside a continuously updated `transcript.md` and
subagent transcripts. Current JetBrains documentation calls the saved state the
full session context, but does not publish the event schema, append/rewrite
rules, model request fields, tool-result coverage, retention, or crash behavior.

Current docs also describe command hooks with useful `UserPromptSubmit` and
`PreToolUse` payloads. That feature is explicitly EAP, not established for the
stable release pinned here. Even in the documented hook contract, there is no
model request/response event and no successful post-tool result. No documented
local telemetry or other complete model-visible leg was found.

A small isolated stable-binary spike is justified, with an early refusal gate:
if stable `3110.7` does not accept the explicit hook config and emit a fresh
workspace/session-bound `SessionStart`, there is no documented independent human
leg. If the session store cannot reconstruct complete model-visible content and
tool results, or if project/auth isolation writes outside both the task roots
and an authorized operating-system temporary boundary, the harness remains
unsupported. IDE, ACP, server, remote and headless behavior cannot fill gaps in
the real terminal TUI.

## Exact product and release boundary

The subject is the standalone JetBrains **Junie CLI**, invoked as `junie` in its
interactive terminal interface ([CLI quickstart]). It is separate from:

- the Junie plugin and AI Chat surfaces inside JetBrains IDEs;
- passive CLI-to-IDE integration, where the CLI discovers a running matching IDE
  and may use IDE-backed tools;
- `--acp` mode, which exposes the CLI engine to an ACP editor/client; and
- headless, server, remote, GitHub/GitLab and SDK-like integration surfaces
  ([IDE integration], [ACP mode], [headless mode]).

The stable production feed current on 2026-09-10 selects technical release
**3110.7** on all six supported platform/architecture pairs. Its non-draft,
non-prerelease GitHub release is named **Junie Release 26.9.7 (3110.7)** and was
published 2026-09-09 at 16:59:30 UTC ([3110.7 release],
[production feed 3110.7]). The annotated tag object
`290631a5e4aab4c943d1efddd16d4dc1d38d6a9a` targets public commit
`ed7f904d75f584d630d86db20cd2cb7da43f8f27` ([3110.7 commit]).

The tag tree contains installers, launch shims, registries, distribution tests
and README/license material; the GitHub release separately attaches binary
assets. Neither publishes the proprietary runtime implementation
([3110.7 tree], [3110.7 release]). The tagged production-feed snapshot stops at
3110.6 ([tagged feed snapshot]); commit
`c2a99694ee9bb8548fb5e1433eda8a828dfa260e` added 3110.7 to `main` about
eight minutes after the 3110.7 release was published ([feed update commit]).
These are distribution facts, not runtime source.

Evidence labels used below:

- **Release-derived** means exact release/tag/feed metadata.
- **Distribution-source-derived** means the tagged open installer/shim code.
- **Documented** means mutable, unversioned JetBrains CLI docs read on
  2026-09-10. They are candidates to measure against 3110.7.
- **Unmeasured** means no stable executable or artifact observation.
- **Runtime-source-derived** conclusions are unavailable.

## Candidate leg 1: saved session files

### What is documented

The interactive quickstart says `Ctrl+O` opens a continuously updated
`transcript.md` stored next to the session's `events.jsonl`. It places subagent
transcripts in the same session's `subagents/` directory and says saved sessions
retain full session context, LLM usage, user prompts and agent responses
([session transcript]). Demo documentation separately uses
`~/.junie/sessions/<session-id>/demo/`. Together with `JUNIE_HOME` replacing
`~/.junie`, this makes
`$JUNIE_HOME/sessions/<session-id>/` the candidate session root; the ordinary
session path still requires direct measurement ([Junie home], [demo artifacts]).

The transcript is a rendered Markdown view documented as continuously updated;
whether it is appended or rewritten is unknown. It cannot be treated as
independent raw evidence from `events.jsonl`. The phrase
“full session context” is a product statement, not a wire contract. Current
documentation does not define:

- `events.jsonl` variants, IDs, roles, timestamps or ordering;
- original versus normalized human bytes;
- system instructions, tool definitions, complete model inputs or responses;
- successful/failing tool-call linkage and the exact result shown to the model;
- subagent parent IDs or how child transcripts are referenced;
- append-only behavior across follow-ups, rewind, branching, compaction, resume
  and process failure;
- large-output spill files, truncation limits or placeholder semantics;
- partial-final-line, inode replacement or flush behavior; or
- general session retention/pruning. Demo artifacts being kept until manual
  deletion does not establish ordinary session retention.

### Capture consequence

A future tailer must remain unarmed until the selected session ID and canonical
workspace are bound. It must then preserve complete raw `events.jsonl` lines by
inode and byte offset, hold a partial tail uncommitted, signal truncation or
replacement, and read-copy the final selected session directory without
rewriting it. It must compare the tail with final bytes and preserve
`transcript.md` only as a derived cross-check.

No rendered transcript, `/logs` ZIP, TUI scrollback or session-history picker can
substitute for the raw event bytes. A parallel foreign session under the same
isolated home must produce zero persisted Goal Cup records.

## Candidate leg 2: command hooks, if stable actually has them

### Availability and loading

**Documented.** The hooks page explicitly says the feature is in Junie's EAP and
directs users to install the EAP CLI ([hooks status]). The current stable docs
are unversioned, and the public 3110.7 tree has no runtime source. Hook support
in stable 3110.7 is therefore **unmeasured**, never assumed.

Default project hooks in `<project>/.junie/config.json` are ignored. Hooks may be
loaded from user `$JUNIE_HOME/config.json`, an explicitly selected
`--config-location` file, or installed extensions. Explicit config remains
enabled for an untrusted project. Matching entries concatenate rather than
override each other ([hook configuration], [configuration trust]).

The smallest future candidate uses one tracker-owned config file selected only
by the process flag. It keeps default config and all default discovery disabled,
uses an empty extensions directory, and watches the selected config plus every
non-secret control file. It never writes app-wide user configuration.

`SessionStart` is non-blocking and hook failure does not abort startup. File
presence or TUI readiness therefore cannot arm capture. The tracker must receive
a fresh authenticated `SessionStart` containing the selected canonical `cwd` and
session ID before accepting the goal. Unsupported hooks, mismatched identity,
timeout, mutation, unexpected merged hook, or config-hash change disarms
capture immediately.

### Payload value and gaps

The current hook reference documents these events:

- `SessionStart` with `session_id`, `cwd`, `project_path` and source
  `startup`, `resume`, `clear` or synthetic `compact`;
- `UserPromptSubmit` with the same identity fields and raw prompt, TUI only;
- `PreToolUse` with tool name and full input;
- `Stop` with final assistant submission text and retry state;
- `StopFailure` with one classified LLM/API failure and details; and
- `SessionEnd` with a termination reason ([hook events], [hook input]).

`PermissionRequest` is unsuitable as an observational capture hook: a matching
successful empty hook automatically approves the action, while a blocking or
failing hook changes behavior. Leave it unconfigured and treat approval/question
input provenance as an open A2 control.

The hook contract has no model request, model response, system-instruction, tool
definition or successful post-tool-result event. `StopFailure` excludes tool
failures, and `Stop` is only the final submission. Hooks alone cannot prove A3 or
complete model-visible content.

Hooks can also mutate behavior: `UserPromptSubmit` may add context or block;
`PreToolUse` may replace input, inject model context, ask or deny; `Stop` can
feed reasons/context back and cause retries. Async context is queued into the
next user submit. The capture handlers must return empty success, except the
dedicated length gate may block a prompt over 4,000 characters after its raw
bytes are acknowledged. No handler may inject context, change tool input,
auto-approve permission, or block completion ([hook mutation],
[hook failure semantics]).

## No documented complete second model leg

No official Junie CLI page found in this preflight documents an OpenTelemetry
file, raw provider request log, or local callback containing full model inputs
and outputs. `events.jsonl` might contain enough data, but its schema is
unpublished. Hooks provide a human/tool-input witness but lack complete model and
tool-result content.

The PTY transcript is a useful lifecycle artifact, not a model-wire leg.
Headless `json-stream` output belongs to a different host, lacks TUI-only
`UserPromptSubmit`, and is only a post-TUI comparison. ACP/server hosts invoke no
hooks in the current hook documentation ([hook host limits], [CLI formats]).

A future result can recommend support only if measured stable artifacts reveal
two independently sourced, session-partitionable legs and at least one
reconstructs every relevant model-visible byte. If both transcript and event
JSON are regenerated from one store, they count as one leg.

## Human versus synthetic content

`UserPromptSubmit` is the best documented positive human witness for ordinary
TUI prompts and real-time follow-ups. The spike must still distinguish:

- the first manually typed goal and later manually typed `keep going`;
- follow-ups queued while the agent is working;
- action approval and agent question replies, for which no observational hook is
  documented;
- `JUNIE_PROMPT` or `--prompt`, which auto-submits after onboarding;
- `/new <prompt>` and `/plan <prompt>` command arguments;
- `!command` entered directly by the user;
- hook `additionalContext` and `Stop` retry observer messages;
- `SessionStart.source=compact` and other compaction summaries;
- main-to-subagent tasks and child-to-main results; and
- custom commands, guidelines, skills, MCP content, IDE context and remote input
  ([real-time prompts], [slash commands], [hook host limits]).

The main qualifying journey must set no initial prompt/task environment or flag,
no remote mode, no ACP, no IDE connection, no MCP server, no extension and no
mutating hook. Prefix and user-role heuristics are insufficient; provenance must
be derived from measured event identity and hook pairing.

## Resume, subagents, models and large output

**Resume.** `--session-id` selects a specific saved session and `--resume`
resumes the last session or the ID supplied with it ([CLI reference]). The
initial run should let the TUI create a session, bind the ID from the fresh
`SessionStart`, then hard-kill the native process tree during a later turn.
Relaunch with `--resume --session-id "$JUNIE_SESSION_ID"` and every original pin.
Require source `resume`, the same event root and a fresh completed turn. Test an
unknown ID separately and prove it does not silently create a qualifying
session. Exact absent-ID behavior, hard-kill recovery and append continuity are
unmeasured.

Switching to a live background session in the same process fires neither
`SessionStart` nor `SessionEnd`, so a connector cannot treat every `/history`
selection as a new arming event. Capture stays bound to one selected ID and
rejects foreign records ([hook host limits]).

**Subagents.** Current docs say a matching subagent works in its own context and
returns a result to the main agent; child transcripts live in `subagents/`.
They do not publish durable parent/child IDs or the exact task/result record
shape. Parts of current subagent configuration are marked EAP, so stable
availability must be measured before A4 ([subagents]).

**Models.** `--model` and `/model` select a primary alias; `/stats` reports used
models. JetBrains says aliases can be remapped and internal summarization,
classification, routing, memory and capability tasks may use a different model
from the same provider ([model selection]). A6 needs requested and resolved
model identity per main, helper and subagent call. A selected alias or aggregate
usage view is insufficient.

**Large output and compaction.** `/compress` or `/compact` compresses context and
may discard information; the EAP hook contract exposes a synthetic
`SessionStart(compact)` ([slash commands], [hook events]). No current docs define
event rewrites, summary linkage, context before/after, tool-output thresholds,
spill paths or truncation. Generate deterministic large output and missing or
non-text tool results, preserve every created file, trigger manual and natural
compaction, and compare the next model input if a recoverable model leg exists.
Rendered previews never establish completeness.

## TUI, IDE, ACP, headless and remote boundaries

The qualifying surface is the real interactive terminal TUI. No positional
task, `--task`, `--prompt`, piped input, `--output-format`, `--json-output-file`
or `--acp` belongs in its main launch.

Junie CLI passively discovers a supported JetBrains IDE opened on the same
project and may connect when IDE-backed features are needed. The docs publish no
disable flag ([IDE integration]). Use a fresh workspace never opened in an IDE;
if the process still reads/connects to app-wide IDE state, record A10 failure.
IDE tools, open-file context, debug mode and plugin behavior are outside this
CLI spike.

Headless tasks are non-interactive, trust project inputs automatically, and do
not emit `UserPromptSubmit`. ACP and server hosts currently emit no hooks.
Remote mode tunnels the same session to a web app and creates another human
input path. Keep all three disabled. Only after the TUI result, one isolated
headless `json-stream` run may compare event vocabulary; it cannot qualify the
TUI connector ([headless mode], [hook host limits], [remote mode]).

## Isolation and authentication

### Verified candidate controls

`JUNIE_HOME` replaces the default `~/.junie`. `--cache-dir` moves caches.
Flags can disable default config, MCP, model, skill, command and agent discovery,
and can redirect the default extensions directory. `--skip-update-check`
disables startup update download; `--share-anonymous-statistics false` disables
statistics sharing for the run ([Junie home], [CLI reference]).

Configuration flags outrank config files. Project config outranks user config,
but `settings.json` uses separate consumer-specific precedence. Interactive
project trust is checked before project inputs load. Choosing **Keep untrusted**
creates no denial marker, keeps project configuration disabled, and leaves
explicit `--config-location` enabled ([configuration trust]).

The same documentation says an untrusted project uses writable temporary
project storage outside the repository and removes it when the CLI closes, but
does not identify that path. A10 must prove that storage stays within an
authorized task root or operating-system temporary boundary; otherwise it
fails.

Trusting a project stores an authentication key in macOS Keychain and a marker
under `$JUNIE_HOME/trust`; no documented switch forces file storage. The future
spike must remain untrusted and use only explicit process-scoped config. If the
stable TUI cannot exercise the required journey that way, refuse instead of
creating an app-wide keychain exception.

The tagged installer hardcodes `$HOME/.local/bin` and
`$HOME/.local/share/junie`, although its generated shim later honors
`JUNIE_DATA` and knows the macOS binary path
`Applications/junie.app/Contents/MacOS/junie`
([tagged installer paths], [tagged shim paths]). Do not run that installer in
the spike. Download the exact architecture asset directly into ignored
`harnesses/`, verify the production-feed SHA-256, extract it into a task-local
root, and invoke the versioned binary directly. Leave system `HOME` unchanged.

### Authentication boundary

JetBrains documents account OAuth for subscription use and a process
`JUNIE_API_KEY`/`--auth` token. The quickstart also describes the Junie API key
as usage-based, while model docs describe the Junie provider as available
through a login or JetBrains AI API key under a subscription. Commercial
entitlement and credential persistence are therefore not safe to infer
([authentication], [model selection]).

The least-state process-token path is contingent. Before any model work, the
operator and observable account/billing evidence must establish that a
`JUNIE_API_KEY` consumes the authorized subscription entitlement rather than
usage-based billing. Only then may the future spike supply it in the process
environment, never on argv. If that cannot be established, do not make a model
request with the token. A separately authorized future probe may instead test
account login entirely within the isolated home without adopting existing
credentials, subject to the same metadata-only and no-global-write gates. If
neither subscription-auth path stays inside the approved isolation boundary,
refuse. Do not use BYOK, configure a custom endpoint, or proxy model traffic.
This preflight authorizes neither auth path.

The hook shell may inherit the process environment. Its command must immediately
exec the capture shim through an allowlist that removes `JUNIE_API_KEY` and all
provider credentials; the shim must never enumerate its inherited environment.
This reduces exposure but is not accepted until measured on the stable binary.

Credential-bearing files receive metadata-only before/after baselines:
existence, file type, mode, size and modification time as available. Credential
bytes, account labels and content hashes are never read. Content hashes are
reserved for non-secret hook and control files. Any keychain check must be
existence-only and separately authorized for the future spike; this research
performed none.

## Smallest future stable A1-A11 spike

### Paths and pins

Create disjoint task paths:

- `JUNIE_SRC`: ignored exact-release asset and extraction root;
- `JUNIE_SPIKE_HOME`: isolated state, data, cache and extensions;
- `W`: fresh tracker-created workspace, never opened in an IDE;
- `CONTROL`: non-secret process config and capture shim; and
- `RAW`: ignored private raw evidence.

None is inside or an ancestor of another, except controlled children of
`JUNIE_SPIKE_HOME`. System `HOME` stays unchanged.

For the machine architecture, download the corresponding 3110.7 macOS asset
from the official release and verify its exact production-feed SHA-256 before
use: `29a70cb48bc156f6dd6b02afe8438c930876c225875ec9c248b37440c70ad92e`
for aarch64 or
`9978a63a92f77eacb02709076f6c3782813f023566229ed909f3aa83ade44fa3`
for amd64. Then invoke:

```text
JUNIE_HOME=$JUNIE_SPIKE_HOME/state
JUNIE_DATA=$JUNIE_SPIKE_HOME/data
JUNIE_CONFIG_DEFAULT_LOCATIONS=false
JUNIE_MCP_DEFAULT_LOCATIONS=false
JUNIE_MODEL_DEFAULT_LOCATIONS=false
JUNIE_SKILL_DEFAULT_LOCATIONS=false
JUNIE_COMMAND_DEFAULT_LOCATIONS=false
JUNIE_AGENT_DEFAULT_LOCATIONS=false
JUNIE_SHARE_ANONYMOUS_STATISTICS=false
```

Only after the process-token subscription-entitlement gate succeeds, also set
`JUNIE_API_KEY=<operator-supplied secret; never logged>`. Otherwise omit the
token and use only a separately authorized, isolated account-login probe, or
refuse before model work.

Use the direct macOS binary:

```text
"$JUNIE_SRC/Applications/junie.app/Contents/MacOS/junie" \
  --project "$W" \
  --cache-dir "$JUNIE_SPIKE_HOME/cache" \
  --config-location "$CONTROL/capture.json" \
  --config-default-locations false \
  --mcp-default-locations false \
  --model-default-locations false \
  --skill-default-locations false \
  --command-default-location false \
  --agent-default-location false \
  --extensions-default-location "$JUNIE_SPIKE_HOME/extensions" \
  --skip-update-check \
  --share-anonymous-statistics false \
  --model "$MODEL_ALIAS"
```

Do not set `JUNIE_PROMPT`/`JUNIE_TASK` or pass an initial task. Do not use
`--brave`, `--acp`, `--resume` on first launch, remote mode, local/custom/BYOK
models, proxy variables, MCP, extensions or an IDE connection. Verify
`--version` before and after outside the measured session, capturing exact
stdout, stderr and exit status. Bind the identifier it actually prints to the
pinned asset SHA-256, production-feed row, tag `3110.7`, and release mapping to
human version 26.9.7. A contradiction is a retained mismatch and refusal; output
that names only one of the two version labels is an observed limitation, not by
itself a mismatch.

The explicit config contains only empty-success observational EAP candidate
hooks for `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `Stop`,
`StopFailure` and `SessionEnd`. It contains the one A7 prompt-length block and no
`PermissionRequest` or mutation output. If stable rejects `hooks` or any event,
retain the error and stop before entering the goal.

### Ordered measurement

1. Snapshot only metadata for documented real-home/config/cache/install/keychain
   surfaces. Hash the non-secret control files. Start the existing raw receiver
   and inode/offset tailer unarmed.
2. Launch the pinned stable TUI in `W`, choose **Keep untrusted**, and require a
   fresh authenticated `SessionStart(startup)` with exact canonical workspace
   and a newly selected session ID. Confirm no IDE match, remote mode, extension,
   MCP, default config or unexpected hook. Stop if arming is unavailable.
3. Manually submit a byte-distinct goal that explicitly writes a file and runs a
   command. Compare the raw prompt across `UserPromptSubmit`, `events.jsonl` and
   transcript, then locate the exact first model input. Do not accept a
   transcript-only match as A1.
4. Submit ordinary and queued `keep going` prompts while the agent works. Exercise
   an agent question and one approval without configuring `PermissionRequest`.
   Test `JUNIE_PROMPT` and `/new <prompt>` only in separate sessions as synthetic
   controls.
5. Run deterministic successful, failing, missing/non-text and large-output
   tools. Join request, result, spill and next model input. Absence of a
   successful model-visible result on an independent leg fails A3.
6. Invoke one documented subagent control. Require durable child raw bytes,
   parent/child IDs, task/result links and model identity; a rendered
   `subagents/` transcript alone does not pass A4.
7. Use a named account-available primary model, complete a turn, switch once with
   `/model`, and force helper work. Require requested/resolved IDs per main,
   helper and subagent call rather than an alias or `/stats` total.
8. Submit a 4,001-character goal. The hook must acknowledge its raw bytes and
   block it before any model call. If stable lacks that gate, record A7 failure;
   never emulate success after dispatch.
9. Submit `also add a README`. Preserve it byte-exactly and classify it as human
   across both legs.
10. Complete a turn, then hard-kill the exact native process tree during the next
    response. Preserve partial raw bytes. Relaunch the same binary with every pin
    plus `--resume --session-id "$JUNIE_SESSION_ID"`. Require fresh
    `SessionStart(resume)`, the same store, a fresh completed turn, and an
    unknown-ID no-new-session failure control.
11. Run a parallel foreign TUI in a different workspace/session under the same
    isolated home. Collect its control evidence separately, but require the
    selected capture path to persist zero foreign records.
12. Trigger manual and natural compaction. Preserve pre-compaction raw events and
    prove how summary/rewrite/spill content reaches the next model input.
13. Exit cleanly. Read-copy the selected session, hook stream, process transcript
    and logs without rewriting. Compare tail/final bytes, account for partial or
    replaced files, then score A1-A11. Run one separate headless
    `--output-format json-stream` comparison only after the TUI result.

Raw artifacts remain only under ignored spike output. Any proposed fixture is a
redacted copy made with the repository tool and reviewed by eye. Raw or
receipt-bound bytes are never rewritten.

## Conditional acceptance after measurement

A `full` recommendation requires two independent, partitionable raw legs;
complete model-visible inputs, outputs and tool results; all TUI human channels;
durable subagent/model lineage; exact-session resume; lossless partial-tail and
compaction handling; continuous arming; and no app-wide write.

`elevated_review` is possible only for a named bounded auxiliary gap after human
provenance and ordinary model-visible traffic are complete. It cannot excuse a
missing second leg, hidden approval/question input, incomplete model/tool
content, foreign persistence, or unknown global credential state.

Refuse when stable hooks are absent, arming fails open, session files cannot
reconstruct model-visible content, the only evidence is transcript rendering,
permission/question replies are unclassifiable, exact resume rewrites
unrecoverably, an IDE/ACP/remote surface contaminates the TUI, or isolation
requires a real-home/keychain write. This preflight assigns no support tier.

## Open questions for the real spike

1. Does stable 3110.7 accept the EAP hook schema and emit every configured event?
2. What exact `events.jsonl` schema, store root and append/rewrite behavior does
   the stable binary use?
3. Do the session files contain system instructions, tool definitions, complete
   model inputs/responses and exact model-visible successful tool results?
4. How are queued prompts, approvals, question replies, `!` commands and
   synthetic prompt sources represented?
5. Can `UserPromptSubmit` block a 4,001-character input before dispatch on stable?
6. How do main/child IDs and subagent task/result/model fields join?
7. Which resolved model handles each main, helper, compaction and subagent call?
8. What happens to event and spill files during large output, compaction, hard
   kill and exact-ID resume?
9. Can the TUI stay untrusted while explicit capture config remains armed and all
   state stays under task roots?
10. Does process-token authentication touch Keychain, existing JetBrains account
    state, system logs or any path outside `JUNIE_SPIKE_HOME`?
11. Can automatic IDE discovery be excluded without an undocumented switch?

[3110.7 release]: https://github.com/JetBrains/junie/releases/tag/3110.7
[3110.7 commit]: https://github.com/JetBrains/junie/commit/ed7f904d75f584d630d86db20cd2cb7da43f8f27
[3110.7 tree]: https://github.com/JetBrains/junie/tree/3110.7
[production feed 3110.7]: https://github.com/JetBrains/junie/blob/c2a99694ee9bb8548fb5e1433eda8a828dfa260e/update-info.jsonl#L352-L357
[tagged feed snapshot]: https://github.com/JetBrains/junie/blob/3110.7/update-info.jsonl#L346-L351
[feed update commit]: https://github.com/JetBrains/junie/commit/c2a99694ee9bb8548fb5e1433eda8a828dfa260e
[tagged installer paths]: https://github.com/JetBrains/junie/blob/3110.7/install.sh#L15-L22
[tagged shim paths]: https://github.com/JetBrains/junie/blob/3110.7/install.sh#L253-L268
[CLI quickstart]: https://junie.jetbrains.com/docs/junie-cli.html
[session transcript]: https://junie.jetbrains.com/docs/junie-cli.html#view-session-transcript
[real-time prompts]: https://junie.jetbrains.com/docs/junie-cli.html#real-time-follow-ups
[CLI reference]: https://junie.jetbrains.com/docs/parameters.html
[CLI formats]: https://junie.jetbrains.com/docs/parameters.html#core-options
[Junie home]: https://junie.jetbrains.com/docs/environment-variables.html#configuration-files
[configuration trust]: https://junie.jetbrains.com/docs/junie-cli-configuration.html#project-trust
[hooks status]: https://junie.jetbrains.com/docs/junie-cli-hooks.html
[hook configuration]: https://junie.jetbrains.com/docs/junie-cli-hooks.html#configure-a-hook
[hook events]: https://junie.jetbrains.com/docs/junie-cli-hooks.html#triggering-events
[hook input]: https://junie.jetbrains.com/docs/junie-cli-hooks.html#hook-input
[hook mutation]: https://junie.jetbrains.com/docs/junie-cli-hooks.html#triggering-events
[hook failure semantics]: https://junie.jetbrains.com/docs/junie-cli-hooks.html#failure-handling
[hook host limits]: https://junie.jetbrains.com/docs/junie-cli-hooks.html#limitations
[subagents]: https://junie.jetbrains.com/docs/junie-cli-subagents.html
[model selection]: https://junie.jetbrains.com/docs/junie-cli-model-selection.html
[slash commands]: https://junie.jetbrains.com/docs/slash-commands.html
[IDE integration]: https://junie.jetbrains.com/docs/junie-cli-jetbrains-ide-integration.html
[ACP mode]: https://junie.jetbrains.com/docs/junie-cli-acp.html
[headless mode]: https://junie.jetbrains.com/docs/junie-headless.html
[remote mode]: https://junie.jetbrains.com/docs/junie-cli-remote-mode.html
[demo artifacts]: https://junie.jetbrains.com/docs/junie-cli-demo.html#output-artifacts
[authentication]: https://junie.jetbrains.com/docs/junie-cli.html#step-3-authenticate
