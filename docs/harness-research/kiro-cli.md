# Kiro CLI capture research — 2.21.0 source preflight

Date: 2026-09-10. This is documentation, release-metadata and public-source
research. No Kiro executable, installer, application, account, model, private
state, keychain or credential store was accessed. The measurement has not been
performed and no support tier is assigned.

## Answer first

Kiro CLI V3 has two plausible local evidence surfaces: its automatically saved
session and project-scoped command hooks. Official documentation gives sessions
UUIDs and a working-directory scope. Hooks can observe prompt submission and
tool use, and a nonzero prompt hook can block before the prompt is processed.
An agent-start event includes a session ID and working directory.

That is enough to justify a guarded experiment, but not a connector. Public
documentation does not define the interactive session database schema, a
lossless append contract, or a byte-exact relationship between hook payloads,
saved history and each model request. It also does not establish hook coverage
for steering, approval feedback, editor replies, slash-command prompts,
compaction or child agents. Kiro's documented home override includes sessions
and configuration, but does not promise relocation of login credentials or all
platform state. No documented native process-tree ownership guarantee was found.

The smallest next step is therefore a fail-closed local V3 TUI experiment. It
must stop before model traffic unless a pinned build, isolated state and
credentials, an owned process lifetime, and an exact workspace/session source
admission have all been established.

## Version, engine and source boundary

The current official CLI changelog identified **2.21.0**, released **September
1, 2026**, as the latest stable Kiro CLI package on 2026-09-10. The same package
can run the V3 engine; “CLI 3.0” names that engine and its incompatible session,
hook and permission formats, not a 3.x package release. The experiment must pin
both package 2.21.0 and V3. If an exact vendor artifact and checksum cannot be
obtained without following a mutable latest channel, the experiment must refuse.

AWS's public Amazon Q Developer CLI repository says it is no longer actively
maintained and that current Kiro CLI is closed source. Its historical code can
explain the old Q product only; it cannot establish current Kiro persistence or
runtime behavior. Kiro's migration guide calls Kiro the next Q CLI update and
retains old `q` entry points and `.amazonq` project configuration. Those
compatibility paths are possible ambient inputs and must be absent from the
isolated test rather than treated as evidence about the new runtime.

## Product boundary

| Surface | Preflight treatment |
|---|---|
| Local interactive Kiro CLI 2.21.0 with V3 | Candidate under test; launch in a fresh workspace without an initial prompt. |
| CLI V1/V2 engines and classic interface | Excluded. Their session, hook and permission contracts differ from V3. |
| Headless `--no-interactive` and `stream-json` | Separate API-key surface; later comparison only. |
| Kiro IDE and Agent Focus | Excluded. Their host owns UI input and some lifecycle behavior. |
| Kiro Web, mobile and `--cloud` sessions | Excluded remote execution and remote session storage. |
| ACP clients | Excluded as the producer under test; an editor owns client-side input. |
| Legacy Amazon Q Developer CLI | Historical compatibility surface, not a substitute for current Kiro CLI evidence. |
| Kiro Crew and asynchronous delegate sessions | Excluded from the first run; exercise one built-in same-harness subagent only. |

## Candidate evidence surfaces

### Leg 1: local saved session

The session guide says local chats are saved after every turn to a SQLite
database under `~/.kiro/`, keyed per directory and identified by UUID. It exposes
exact-ID resume and reports that a session cannot be active in two processes.
`KIRO_HOME` is documented to relocate sessions and global Kiro configuration.

The same documentation does not name the database file, row grammar, transaction
or append behavior. The ACP page separately names per-session JSON and JSONL
files under `~/.kiro/sessions/cli/` and says the TUI uses ACP internally, while
the session guide calls the TUI store SQLite. This is not a reliable public
mapping from an interactive session to original bytes. V3 migration documentation
also says its session format is incompatible with V2 and that importing complex
history may lose tool outputs.

The tracker must discover the actual files only inside the isolated `KIRO_HOME`,
then bind them to the fresh session ID and canonical workspace before accepting
content. A UUID, directory key or path alone is insufficient. SQLite WALs,
replacement files, partial writes and any ACP JSONL must be captured as candidate
raw sources until a final-file comparison proves their roles.

### Leg 2: project command hooks

V3 hook files live at `.kiro/hooks/*.json`. The current overview and migration
guide call the lifecycle event `SessionStart`, while the trigger reference calls
the CLI event `Agent Spawn` and shows an `agentSpawn` payload. That payload has
`hook_event_name`, `cwd` and `session_id`; a tool example adds `tool_name` and
`tool_input`. Prompt text is exposed to shell actions through `USER_PROMPT`. A
nonzero command blocks Prompt Submit and Pre Tool Use, while other hook errors
only notify the agent/user. Successful hook stdout is added to agent context, so
the tracker hook must emit no stdout.

These pages do not publish one complete, consistent event-envelope schema or an
exact post-tool-result field contract. Project and global hooks merge, and hooks
may also arrive through an agent profile or cloud configuration. File presence
is therefore not arming evidence. Before the goal, an isolated no-model launch
must settle the effective V3 lifecycle trigger, and the owned loopback receiver
must see it with the canonical `cwd` and fresh session ID. The effective
configuration must exclude global, agent and cloud-synced hooks. A missing or
ambiguous lifecycle event, or missing later hooks, is a visible refusal.

### Debug and rendered surfaces

`KIRO_ACP_RECORD_PATH` can record TUI ACP traffic to JSONL, and headless mode can
emit stream JSON. These are useful experiment controls, not documented complete
raw/model legs. `/transcript` renders or exports the conversation and the session
dashboard indexes local V2/V3 user prompts, but rendered/exported/indexed views
may omit or transform content. None can replace final persisted-source checks.

## Human and model-visible content

An ordinary typed prompt is the first candidate join between the saved session
and Prompt Submit. Each of these documented channels needs its own marked control
because no source establishes that it passes through the same hook and history
records with unchanged bytes:

- steer messages can arrive mid-turn, while queued messages are sent as later
  prompts and multiple steer messages may be concatenated;
- `/editor` and `/reply` submit editor content, including quoted assistant text;
- approval prompts accept allow, deny and drill-down feedback;
- slash commands can switch agents/models, add context, spawn sessions or invoke
  skills, and their generated prompts may not be direct human text;
- `@path` expands file content before sending and truncates large files;
- image paste and voice input are distinct non-text/transcribed channels; and
- `!command` executes locally without the model and is a participant workspace
  action, not an agent tool call.

The future test must keep human typed bytes separate from expanded files,
approval/tool protocol, hook stdout, steering files, skills, model-generated
compaction summaries and synthetic child prompts. Any permitted model-visible
human input without a byte-exact two-leg join is a refusal.

## Tools, compaction, models and children

Post Tool Use is documented as having access to tool results, but no durable
size, binary, truncation or spill contract is published. TUI display collapsing
does not prove persisted completeness. The measurement needs success, failure,
non-text and large-output cases and must compare the next model request with all
candidate persisted bytes.

Manual and automatic compaction summarize older messages. The context guide says
compaction creates a new session and the original remains resumable. The exact
old/new link, summary bytes, model identity and persistence transition are not
specified. The test must retain both IDs and refuse if it cannot prove the join.

The built-in subagent runs with isolated conversation context while sharing the
workspace; the main agent waits for completion and aggregates its result. Public
documentation does not identify a durable child transcript or a parent/child ID
pair. A valid experiment needs child raw bytes, both-direction linkage, the
parent tool call/result, and resolved model identity. `/spawn`, delegate, Crew
and cloud agents are separate lifecycle surfaces and remain off.

Model selection can be changed in-session and `Auto` may route tasks. A saved
preference or UI label is not the resolved model for a request. The test needs a
per-call or per-message resolved identity for the parent, compaction and child;
otherwise model attribution remains unknown.

## Stop, resume and process ownership

`kiro-cli chat --resume-id <UUID>` is the only candidate resume form. “Most
recent,” pickers and cloud-auto-detection are unsafe for the trial. The launcher
must verify that the exact local ID resumes the intended workspace/history or
fails without creating or attaching another session. Compaction successor IDs
must be followed explicitly.

No official source found in this pass promises that terminating the TUI stops
all hooks, tools, MCP servers and subagents. Before authentication or model
traffic, the launcher must start the no-prompt CLI in an owned native process
group, inventory descendants of that exact process, interrupt and then terminate
the group within a bound, wait its members, and confirm that no owned listener or
writer survives. Failure ends the experiment. The measured run repeats this
mid-turn and preserves partial raw evidence before exact-ID resume.

## State, policy and authentication

Kiro merges global, project and agent configuration. Project agents override a
same-name global agent; steering, skills and hooks merge; permissions use a
deny-overrides rule. V3 also supports cloud configuration in new local sessions.
Therefore a project hook cannot by itself prove isolation or resolved policy.
The future run must use an otherwise empty `KIRO_HOME`, a fresh project, no
legacy `.amazonq`, no cloud configuration, no global/agent hooks, no MCP servers,
and a restrictive permission policy whose effective result is inspected before
the goal.

Interactive CLI authentication uses browser or device-flow login through GitHub,
Google, Builder ID, IAM Identity Center or an organization provider. A process
`KIRO_API_KEY` is for non-interactive use and has lower precedence than an active
browser session. Headless execution is billed through subscription credits, but
it is still a different input and approval surface and cannot substitute for the
interactive subscription trial.

`KIRO_HOME` is documented for Kiro resources, settings and sessions, not for all
login tokens, OS keychain entries or installer/updater state. The experiment
must use an explicitly authorized test account and fresh harness HOME/XDG roots,
then compare credential-bearing locations by existence, type, mode, size and
modification time only. It must never read or hash credential bytes. Refuse if
the login touches the real home/keychain, cannot be relocated, inherits an
existing browser session, or forces cloud/IDE/remote operation.

## Smallest future isolated measurement

1. Obtain an exact vendor Kiro CLI 2.21.0 package in ignored `harnesses/`, verify
   vendor metadata/checksum, disable updating, and invoke the pinned executable
   directly. Refuse if only a mutable latest artifact is available.
2. Create disjoint harness HOME, XDG, `KIRO_HOME`, workspace, receiver and raw
   roots. Before login, prove no-prompt V3 starts under the expected engine and
   that its owned process group terminates without a survivor.
3. Authenticate an explicitly authorized subscription account without importing
   existing state. Use metadata-only credential baselines and refuse unless all
   resulting state remains isolated.
4. Exclude legacy Q configuration, cloud sync/sessions, IDE/ACP clients, global
   hooks, custom agents, MCP, skills and external tools. Install only the project
   tracker hooks and a restrictive project permission policy.
5. Arm only after the no-model launch resolves the documented lifecycle-trigger
   ambiguity and the loopback receiver gets that fresh event with the canonical
   workspace and session ID. The tracker must also admit regular raw sources
   beneath isolated `KIRO_HOME`; any mismatch or pre-goal model traffic is a
   refusal.
6. Run the standard script plus steer/queue, editor/reply, approval feedback,
   `@path`, direct-shell negative, compaction, one built-in subagent, large/error
   tool output, model switch and a parallel foreign workspace/session. Stop on
   the first foreign persisted byte.
7. Kill the owned process group mid-turn, preserve partial sources, and resume
   only the exact session/successor ID with the same engine and pins. Require a
   matching fresh hook before accepting later bytes.
8. Compare captured tails with final database/WAL/JSONL sources and optional ACP
   debug recording. Redact only reviewed fixture copies. Run headless mode later
   as a separately labelled comparison.

## Conditional decision after measurement

- `full` requires two complete partitionable legs, every interactive human
  channel, lossless tool/model content, durable child lineage, resolved models,
  exact resume/compaction, continuous arming and no global or remote side effect.
- `elevated_review` could cover only a named bounded auxiliary gap after complete
  human provenance and ordinary model traffic are established.
- Refuse for invisible human input, an incomplete or mutable-only raw source,
  suppressible/misattributed hooks, foreign persistence, unresolved child/model
  identity, unsafe process ownership, non-isolated credentials/state, an
  unpinned build, or forced cloud/IDE/remote behavior.

## Official sources

All pages and metadata were checked 2026-09-10.

- [Kiro CLI changelog](https://kiro.dev/changelog/cli/) and
  [CLI 3.0 engine overview](https://kiro.dev/docs/cli/v3/)
- [Session management](https://kiro.dev/docs/cli/chat/session-management/),
  [ACP persistence](https://kiro.dev/docs/cli/acp/), and
  [V3 migration guide](https://kiro.dev/docs/cli/v3/migration-guide/)
- [Hooks overview](https://kiro.dev/docs/hooks/),
  [trigger inputs](https://kiro.dev/docs/hooks/types/), and
  [hook actions](https://kiro.dev/docs/hooks/actions/)
- [Queue steering](https://kiro.dev/docs/cli/chat/queue-steering/),
  [context and compaction](https://kiro.dev/docs/cli/chat/context/),
  [terminal UI](https://kiro.dev/docs/cli/terminal-ui/), and
  [file references](https://kiro.dev/docs/cli/chat/file-references/)
- [Subagents](https://kiro.dev/docs/custom-agents/subagents/) and
  [model selection](https://kiro.dev/docs/cli/chat/model-selection/)
- [Configuration scopes](https://kiro.dev/docs/cli/chat/configuration/),
  [permissions](https://kiro.dev/docs/cli/chat/permissions/), and
  [settings](https://kiro.dev/docs/cli/reference/settings/)
- [Authentication](https://kiro.dev/docs/cli/authentication/),
  [headless mode](https://kiro.dev/docs/cli/headless/), and
  [enterprise governance](https://kiro.dev/docs/enterprise/governance/)
- [Upgrade from Q CLI](https://kiro.dev/docs/cli/migrating-from-q/) and the
  [historical AWS Amazon Q CLI repository](https://github.com/aws/amazon-q-developer-cli)
