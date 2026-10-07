# Factory Droid CLI capture research — v0.216.0 source preflight

Date: 2026-09-10. This is documentation, metadata and public-source research.
No Factory executable, installer, application, account, model, user state,
keychain or credential store was accessed. The A1–A11 measurement has not been
performed and no support tier is assigned.

## Answer first

Current official sources support only a guarded measurement.

It is not enough to justify a connector. Factory does not publish the persisted
JSONL grammar or a completeness/append contract. The documented local-session
root is incomplete. The CLI has human interaction paths whose relationship to
the two candidate legs is unknown, including side questions, queued steering,
permission comments, questionnaires and voice input. No public
contract establishes relocatable CLI state or credentials, a durable child-parent
transcript join, lossless large results, or native process-tree ownership.

The next step is therefore a fail-closed, isolated macOS TUI experiment. It must
stop before model traffic unless state and credentials remain isolated, the
tracker owns the process lifetime, cloud/IDE/remote continuations are off, and
the admission rules below are satisfied.

## Version and source boundary

On 2026-09-10 the npm `latest` tag for both `droid` and `@factory/cli` resolved
to **0.216.0**, published at 00:12 UTC. The Darwin platform packages carried the
same version. The metadata recorded these integrity values without fetching a
tarball:

| Package | npm SHA-512 integrity |
|---|---|
| `droid` | `sha512-62YviOZrfGCabQro2zEeSrsrQfbRF8MeFRDm5gqg7d4gjRWQ2kO1C+8jIdzgUM8mc/rCZo+5g77TnlXy2Vxokg==` |
| `@factory/cli` | `sha512-qZTikEYrNEelqaZZlpTelOk8ZgJTWLtLxtQDV0noQ0mnSqjvt0FGe5rjtoSJ9GTbXVNdrNA1CcIS82jYtrdN1w==` |
| `@factory/cli-darwin-arm64` | `sha512-Zjs4dseXh/Wjs4rGhtH+MeHxFGLUrjoQnRn3w0n0iACAYK1Weiwxc1e+8CxYreEO98r8auf3lDtrscStjKsqCw==` |
| `@factory/cli-darwin-x64` | `sha512-+mMZ8Fwb+lV9HX+m7i8SAME34hZLCQ3LgJ+M8N2gTz5LkU6YJ+8jpFqOxda3nOeRzN9CIjF+4hNNZMkSCYcwQw==` |

The official changelog still identified CLI 0.209.0 from 2026-09-01 as its
newest entry when checked. The npm release metadata is the candidate-version
authority for this preflight; the mutable documentation may describe a newer or
older build and every behavior still needs confirmation on 0.216.0.

Factory's public `factory` repository was read at
`eb4c4e381fd708b704bdf072d8560e9addbf5d06` (2026-09-03). It contains public
documentation and distribution pointers, not the Droid CLI runtime. The public
TypeScript SDK repository was read at
`87b4c4c4e4fc093e10a94f1e767ed57b7cbf597e`. Its typed protocol and examples are
useful candidates, but they are not evidence for the interactive TUI's
persisted bytes.

## Product boundary

| Surface | Preflight treatment |
|---|---|
| Interactive local `droid` TUI | Candidate under test. Start in a fresh workspace with no initial prompt. |
| `droid exec` and stream JSON-RPC | Separate headless surface; at most a later comparison after TUI evidence. |
| Factory App and web/mobile sessions | Excluded. Session mirroring and remote input would add surfaces outside the local TUI. |
| IDE integrations and ACP | Excluded. Their host owns parts of history and input handling. |
| SDKs, public API and daemon clients | Excluded as producers. Their schemas can help interpret observed fields only. |
| Droid Computers, BYOM and cloud agents | Excluded remote execution. |
| Missions | Excluded from the first measurement; use one same-harness built-in Task subagent instead. |

## Candidate evidence surfaces

### Leg 1: local saved session

The Hooks guide gives an example `transcript_path` under `.factory/projects/...`.
The pinned TypeScript SDK exposes `listSessions({ cwd, limit })` and says it reads
local session storage, but does not document that storage root. Neither source
publishes the interactive session JSONL schema, record ordering, write/rewrite
behavior, spill layout or a promise that persisted content equals every
model-visible byte.

Treat the event-supplied transcript path as a candidate. Admit only a regular
descendant of the isolated root when the event matches the expected workspace
and session; otherwise stop before model traffic. A tailer must prove
append/replacement/partial-tail semantics against final files rather than
assuming JSONL.

### Leg 2: project command hooks

Project `.factory/hooks.json` receives `session_id`, `transcript_path`, `cwd`,
`permission_mode`, `hook_event_name` and an optional `message_id`. Prompt events
add `prompt` and `has_images` and may block. Tool events add tool name, input and
response. Lifecycle events distinguish startup/resume/clear/compact, compaction
and child stop. Hooks snapshot at startup; global disablement and
`allowManagedHooksOnly` may suppress project hooks.

The tracker command must not add context or alter tools; only the length gate may
block. Arm on a fresh matching startup event at the owned endpoint; event loss is
a capture-health failure.

### Optional content telemetry

Customer OpenTelemetry can emit user/assistant content and tool calls/results to
a configured collector, with session and active-model attributes. It is off for
content by default, can be overridden by organization policy, and truncates any
single content attribute at 32,000 characters. It is therefore an optional
cross-check for model identity and small records, never the raw or complete leg.
It may be tested only with an owned loopback sink that is effective before the
goal; an organization-pinned non-loopback content sink is an early privacy
refusal.

## Human and model-visible content

The ordinary typed prompt must join on both candidate legs. The following need
explicit controls because the docs establish the UI surface without establishing
the same join:

- `/btw` asks a model side question outside the main transcript view;
- ordinary queued steering is a TUI control; edit-before-send is documented for
  Factory App and is outside this measurement;
- permission responses can carry a comment, and AskUser answers are strings;
- voice dictation is organization-controlled;
- custom commands can add model-visible instructions; and
- bash mode executes human shell text without model interpretation.

The real measurement must exercise each permitted human channel and forbid a
tracked run if any model-visible input lacks a byte-exact human marker on the two
admitted legs. Bash-mode mutations are also workspace changes by the participant;
they cannot be treated as agent tool calls. Harness-authored control output,
compaction summaries, skill bodies, system notifications, retries, tool results
and subagent task prompts must not be classified as typed human messages merely
because they occupy a user-like slot.

The public SDK's live stream distinguishes user, assistant, tool call, tool
result, hook, error and result messages. That describes an SDK stream, not the
TUI file. TUI resolved-model evidence is absent. Model switching,
router/fallback selection, compaction-model calls and subagent models must be
joined to persisted per-call or per-message evidence. An active setting or
display label alone is insufficient.

## Tools, compaction and subagents

TUI tool-result type, size and spill are undocumented.
Content telemetry's 32,000-character cap makes it a deliberate lossy
control. The spike needs
successful, failed, non-text and large tool cases, then must compare the next
model request with the transcript, project event and any observed spill.

Manual and automatic compaction, including `/compress`, need an observed
old/new-session join. Public SDK replacement APIs return a successor session,
but do not establish the TUI's preserved history, summary bytes, file replacement
or tail behavior.

Built-in `worker` and `explorer` subagents run through the Task tool in fresh
contexts. Task accepts a `resume` task ID and background tasks expose stop/output
controls. No public local-file contract shows a durable child transcript with
both parent and child identifiers. The spike must prove the parent Task
call/result, child bytes, both-direction join, resolved model and behavior across
parent resume.

## Stop, resume and process ownership

Interactive `droid --resume <session-id>` is the only candidate resume command.
The form without an ID selects the latest session and must not be used. The test
must establish that a full exact ID either resumes the intended history or fails
without creating a substitute, and must follow compaction/fork IDs explicitly.

No official source promises that terminating the interactive process terminates
its daemon, tools and subagents, or identifies every descendant. Before
model traffic, the launcher must start the no-prompt TUI in an owned native
process group, inventory only descendants of that exact process, interrupt and
then terminate the group within a bounded interval, wait every child, and show
no owned listener or writer survives. Failure ends the experiment. The real A5
run repeats this mid-turn and preserves partial bytes before exact-ID resume.

## State, policy, cloud and authentication

Personal settings live at `~/.factory/settings.json`; project and nested folder
settings merge with user, runtime and organization sources. Cloud session sync
defaults on, IDE auto-connect can be enabled, and organization policy can force
model, sync, retention,
telemetry and subagent choices. `remoteAccessEnabled` defaults off but still needs
an effective-state check. Local files alone cannot prove the resolved policy.

No official source found in this pass documents one environment variable that
relocates all CLI sessions, caches, settings, plugin/skill discovery, OAuth state
and keychain use. The future spike must use a fresh harness home and XDG roots,
then prove with metadata-only before/after snapshots that the real home, system
settings and keychain are untouched. It must set cloud sync, remote access and
IDE auto-connect off and refuse if higher policy prevents those effective values.

The interactive quickstart uses browser OAuth. Factory says OAuth tokens are
encrypted, rotate, and have OS file permissions, but does not publish their path
or relocation control. Factory also supports a process `FACTORY_API_KEY` and
individual subscription plans include the CLI and SDK under rolling limits. The
public material does not establish whether API-key execution consumes the same
included subscription allowance or another billing path. The Goal Cup trial
must use an explicitly authorized participant subscription flow whose credential
storage passes isolation; it may not silently substitute a key, BYOK model,
custom endpoint or usage-paid automation credential.

## Smallest future isolated measurement

1. For the host architecture, fetch the exact direct binary at
   `https://downloads.factory.ai/factory-cli/releases/0.216.0/darwin/<arch>/droid`
   and the checksum at
   `https://downloads.factory.ai/factory-cli/releases/0.216.0/darwin/<arch>/droid.sha256`
   into ignored `harnesses/`. Verify that binary against the vendor SHA-256 and
   invoke it directly. The npm integrity values above authenticate npm tarballs,
   not this file. Do not run the installer or updater.
2. Create disjoint harness home, XDG, workspace, tracker-control and raw-output
   roots. Disable auto-update. With no prompt and no authentication, prove that
   state stays under those roots and that the owned process group terminates
   without descendants. Refuse on any unexplained real-home, system or keychain
   access.
3. Authenticate an explicitly authorized subscription account without adopting
   existing credentials. Snapshot credential-bearing locations by existence,
   type, mode, size and modification time only; never read or hash their bytes.
   Refuse if login cannot remain isolated or if API-key billing is the only path.
4. Set local-only sync, no remote access, no IDE auto-connect, no custom models
   and no external MCP/connectors. Apply the admission and refusal rules above.
5. Start the real TUI with no initial prompt. Arm only after satisfying those
   rules, then tail only the admitted file.
6. Run the canonical script plus `/btw`, queued steering, permission/AskUser,
   direct-bash negative control, voice-disabled evidence, manual/automatic
   compaction, a built-in Task child, model switch, large/missing/error tool
   results and a parallel foreign workspace/session. Stop on the first foreign
   byte admitted to Goal Cup storage.
7. Kill the owned process group mid-turn, preserve raw partial files, resume only
   the exact session ID with every pin, and reapply admission before accepting
   more bytes. Compare the tail against final files and optional
   loopback content telemetry. Redact only reviewed fixture copies.
8. After the TUI result, run one separately labelled `droid exec` comparison. It
   cannot substitute for interactive evidence.

## Conditional decision after measurement

- `full` would require two partitionable complete legs, every TUI human channel,
  lossless model/tool content, durable child lineage, resolved models, exact
  resume/compaction, continuous arming and no global or remote side effect.
- `elevated_review` could cover only a named bounded auxiliary gap after human
  provenance and complete ordinary model traffic are established.
- Refuse for an invisible `/btw`, question, queued or voice input; an incomplete
  or mutable-only transcript; suppressed arming; foreign persistence;
  unresolved child or model identity; unsafe process ownership; non-isolated
  credentials/state; or forced cloud/IDE/remote operation.

## Official sources

All pages and metadata were checked 2026-09-10.

- [npm `@factory/cli` metadata](https://registry.npmjs.org/%40factory%2Fcli)
  and [Darwin arm64 package metadata](https://registry.npmjs.org/%40factory%2Fcli-darwin-arm64)
- [Factory full changelog](https://docs.factory.ai/changelog/release-notes)
- [Droid CLI overview](https://docs.factory.ai/droid-cli/overview),
  [quickstart](https://docs.factory.ai/droid-cli/quickstart), and
  [CLI reference](https://docs.factory.ai/droid-cli/cli-reference)
- [Hooks](https://docs.factory.ai/harness/hooks) and
  [custom droids](https://docs.factory.ai/harness/subagents)
- [Settings](https://docs.factory.ai/droid-cli/settings) and
  [enterprise settings resolution](https://docs.factory.ai/enterprise/hierarchical-settings-and-org-control)
- [Telemetry](https://docs.factory.ai/enterprise/telemetry),
  [privacy controls](https://docs.factory.ai/enterprise/telemetry/privacy), and
  [data reference](https://docs.factory.ai/enterprise/telemetry/data-reference)
- [Security](https://docs.factory.ai/enterprise/security) and
  [individual plans](https://docs.factory.ai/pricing/individuals)
- [Droid Exec](https://docs.factory.ai/droid-exec/overview),
  [IDE integrations](https://docs.factory.ai/ide-integrations), and
  [Droid Computers](https://docs.factory.ai/droid-computers/overview)
- [Factory public repository at `eb4c4e3`](https://github.com/Factory-AI/factory/tree/eb4c4e381fd708b704bdf072d8560e9addbf5d06)
- [TypeScript SDK repository at `87b4c4c`](https://github.com/Factory-AI/droid-sdk-typescript/tree/87b4c4c4e4fc093e10a94f1e767ed57b7cbf597e)
  and [SDK reference](https://github.com/Factory-AI/droid-sdk-typescript/blob/87b4c4c4e4fc093e10a94f1e767ed57b7cbf597e/docs/typescript-sdk-reference.md)
- [Python SDK repository at `6d06b4e`](https://github.com/Factory-AI/droid-sdk-python/tree/6d06b4e613aab3990cf2ced469b5c4e80053e52e)
