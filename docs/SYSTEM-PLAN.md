# The /goal Cup — System Plan and Delivery Tracker v0.2

> **Historical (2026-10-02).** Section 4 predates M1 and the M2 recorder; public V1 scope is
> recorded in [STATUS](../STATUS.md#v1-on-v1trunk); sections 5-7
> stay live. Current: [ARCHITECTURE](ARCHITECTURE.md), [STATUS](../STATUS.md).

Status: **section 4 (workstream A) is history** — it is the tracker plan as it stood before M1 was built, and `docs/PRD-tracker.md` plus the code supersede it wherever they disagree. Sections 5 to 7 (backend, website, compliance) are live and are covered nowhere else, which is why this file is kept whole. Supersedes v0.1.
Updated: 2026-09-03; scoped as above 2026-09-08
Target event: late September or early October 2026; exact T0 is not set.

v0.2 rewrites the tracker workstream after the harness research pass (`docs/harness-research/`) and the design review recorded in `docs/DECISIONS.md`. The competition model in section 2 is unchanged except where a decision explicitly touches it. Workstreams B, C, and D carry over with the deltas noted.

### M0 outcome (2026-09-03)

Milestone M0 is done. Nine capture spikes were run, one per harness and mode, with dsh prepared but never started. The measured results are in [M0 findings](M0-FINDINGS.md), the [public write-up](CAPTURE-RESEARCH.md#21-nine-spikes-one-per-harness-and-mode), and the included redacted fixtures described in its [public file map](CAPTURE-RESEARCH.md#71-where-things-live). Detailed per-harness reports are intentionally omitted from this public OSS cut.

Nothing in the shape of the design changed. The daemon still records raw and the verifier still interprets, there are still two legs per harness, and configuration is still folder-scoped. Five things sharpened, and they are the reason this document was revised: arming became a hard gate before the clock starts (D16), the daemon's short list of things it judges grew from two signals to five (D17), the human-message rule became two-legged with a per-connector exclusion list because no single positive marker survived contact with any harness (D18), leg-1 stores turned out to be home-wide almost everywhere and now have to be partitioned at capture time (D20), and folder scoping gained four named app-wide exceptions rather than staying absolute (D21). The measured tier per harness is D23: three at `full`, five at `elevated_review`, and dsh unsupported for v1.

## 1. Delivery strategy

Four workstreams with stable contracts between them:

1. **Open-source tracker daemon, connector manifests, and verifier** — first priority.
2. **Agent Community Auth and event backend** — open source, operated by the Agent Community team.
3. **Participant website and public race board** — operated by the Agent Community team.
4. **Compliance, research, publication, and judging operations** — organizer-only operation of open-source tooling.

The tracker develops against the public protocol specification, signed test fixtures, and a mock event server. The verifier is one crate used by the backend, by CI, and by participants offline.

## 2. Locked competition model (deltas from v0.1)

- The workspace is empty except for the tracker's own folder-scoped configuration files, which are declared, watched, excluded from the sealed manifest, and disclosed on the public entry.
- The first message typed into the fresh session is the original prompt. `started_at` is signed when the daemon observes that message.
- `keep going` may arrive through any channel. Content is checked, not channel. Cloud and remote execution modes are ineligible.
- The tracker never injects anything into the agent's context.
- One harness per entry. Subagents inside the same harness only.
- The seal is the workspace manifest taken at the deadline.
- Harness versions must fall inside the connector's tested range at start; updating during the run is prohibited and recorded.

Everything else in v0.1 section 2 stands.

## 3. Run lifecycle

Unchanged from v0.1: entry states `running`, `sealed`, `published`, `verified`, `incomplete`, `ineligible`, `judging`, `judged`; observed health `online`, `upload_lagging`, `connection_lost`, `agent_exited`, `tracker_interrupted`, `deadline_reached`.

Added health states: `capture_stalled` (harness process alive, expected transcript not growing), `hooks_silent` (transcript growing, no hook posts), `config_restored` (tracker config in workspace deviated and was rewritten).

M0 added four more, and they complete the list of things the daemon is allowed to judge (D17): `arming_pending` (the workspace is armed but the connector's start signal has not arrived, so the clock has not started), `arming_lost` (a hook file was restored on a harness where trust is a hash of the definition, so capture may be degraded until the participant re-arms), `config_mismatch` (a shim reported that its pinned hook-config hash no longer matches, which is how a hot-reload deletion surfaces), and `version_changed` (the harness updated itself mid-run). `hooks_silent` is no longer a blanket rule: each connector declares, per event, whether silence is expected, conditional or never normal, with "leg 2 silent while leg 1 grows" as the universal fallback.

## 4. Workstream A — tracker daemon, connectors, verifier

> **Superseded, and kept as evidence.** This section is the design as it was written on 2026-09-03, before any of it was built. `docs/PRD-tracker.md` is the requirements document and the tree is the build; where this section and either of those disagree, this one is wrong. Read it for why the shape was chosen, not for what exists. Individual corrections below are dated inline.

### 4.1 Principle

Record raw, interpret later. Every supported harness writes a complete, model-visible transcript to local disk. The daemon tails, hashes, encrypts, and ships those bytes. It understands harnesses only through declarative manifests. All parsing and rule checking happens in the verifier, server-side or offline, where a fix is a re-run rather than lost evidence.

M0 confirmed this for a reason we had not anticipated. Three of the seven research files were wrong about the record type that identifies a human message on the versions we tested. Had that rule shipped inside the daemon, those entries would have been mis-verified with no way back. It also drew one honest boundary around the claim: every token the agent saw is captured, but not every byte a tool produced, because several harnesses truncate with the untruncated bytes existing nowhere on disk.

### 4.2 Components

**Daemon** (Rust, one binary, one supervised service per machine):

- device authorization and scoped grant storage; *(corrected 2026-09-08: this is the CLI's, not the daemon's — `goalcup auth` writes `grant.json` at mode 0600 under the config directory)*
- entry registry: workspace path, harness, session binding, receipts, per-run keys;
- workspace creation and zero-entry baseline (tracker config files excluded), with the tracker's own artifact root deliberately placed outside and above nothing: M0 watched agents enumerate the parent directory on their first tool call and read the tracker's settings, receiver log and hook stream out of it;
- launch (CLI mode) or arm (desktop attach mode) per manifest recipe, followed by the arming gate: the daemon waits for the connector's declared start signal for the tracked workspace before it will ask the server to stamp `started_at`;
- tailers: append-only JSONL, directory-of-files, SQLite, each read through the cursor type its store declares rather than a generic sequence, and each partitioning its rows at capture time so that home-wide stores never ship another session's bytes;
- local hook endpoint (localhost HTTP and Unix socket) receiving posts from harness hooks or forwarding shims, routed by working directory and authenticated with a per-run token, because an unauthenticated loopback receiver in M0 accepted a post from another spike's leftover shim;
- config watch: expected hashes for every tracker file in the workspace, the shim's own pinned hook-config hash, and a named allowlist of app-wide files whose deviation is meaningful; deviation event with diff, immediate restore, and an `arming_lost` state where restoring the file invalidates the harness's hook trust;
- pattern redaction with redaction log;
- per-line hash chain, sequence numbers, per-run encrypted write-ahead buffer (`redb` + `age`), continuous upload, server acknowledgements, catch-up after outage with a visible "back online, N events uploaded" state;
- watch of all installed harness stores during a run for sessions inside a bound workspace that belong to a different harness (flag);
- signed local status cache; `goalcup status --json`;
- seal at deadline: manifest first, excluding both the tracker's config and the paths the harness itself creates in the workspace, then termination following the connector's kill strategy, because on at least one harness killing the launcher leaves the real session running and holding its writer lock;
- generous outage thresholds; reconnection is the normal path, not an error.

**CLI**: `goalcup auth`, `goalcup start --harness <id>`, `goalcup status [--json]`, `goalcup ui`, `goalcup entries`, `goalcup resume <entry>`, `goalcup seal <entry>`, `goalcup doctor` — eight sub-commands. *(Corrected 2026-09-08: this line listed seven and included `verify`. Verification is a separate binary, `goalcup-verify <trace-dir>`, so the daemon never links the verifier — D3; `ui` and `seal` were missing.)*

**Agent-facing extras, shipped but participant-registered**: a read-only MCP server (`goalcup_event_status`, `goalcup_run_status`, `goalcup_entries`) reading the signed cache, and a small skill describing when to ask.

**Connector manifest** (data, no code): harness id and family; detection (binary, version command, tested version range); launch recipe (argv, env, flags, dedicated home or profile, how the prompt reaches the session); arm recipe for desktop modes (which folder-scoped config to write, how a new session is recognized); stores to tail (path templates, tail mode, sequence key, subagent and spill locations); hook install recipe (folder-scoped files, forwarding shim if the harness uses plugins rather than command hooks); blocking prompt-length hook where supported; human-message rule reference for the verifier; capture tier (`full`, `elevated_review`).

M0 proved fifteen further fields necessary, each traceable to something measured rather than imagined; the full list with its evidence is PRD section 5.4 and `CAPTURE-RESEARCH.md` 5.6. The ones that change how the daemon behaves rather than how the verifier reads are the arming signal and its recipe, whether the start signal is required at all, the cursor type and unsafe cursors per store, the partition key per store, the kill strategy and lock paths, the config allowlist and the declared app-wide exception, the hook-silence policy with its hot-disable window, and the receiver token. The ones the verifier needs are the human-rule id with its exclusion list and byte-exact field, the trailing-newline normalisation, the leg join key, the workspace exclusions, and the archive sources. *(Corrected 2026-09-08: the auxiliary-call allowlist was in this list. `aux_calls[]` was removed from schema v0 in the M1a fix wave and a protocol test holds it absent from every manifest; PRD 5.4 records why. PRD 5.4 also carries six fields added after this list was written.)*

**Forwarding shims**: tiny plugins for plugin-based harnesses (OpenCode TypeScript, Hermes Python, OpenClaw TypeScript, dsh Cordis) whose only job is to post hook payloads to the daemon endpoint. Shipped in the release, hashed in the manifest.

**Verifier** (Rust crate, open source, runs in the backend and as the `goalcup-verify` binary):

- per-harness parsers from raw stores to ATIF v1.8 with hash links to raw files in `extra`;
- per-harness human-message rules, each a positive test plus an ordered exclusion list of harness-authored user-slot content plus scoping clauses plus corroboration on both legs (Claude Code: `origin.kind=human` on a main-session record, corroborated by the prompt hook and the raw request bodies, with suggestion-agent payloads never feeding the list; Codex: a user-role message in the file whose first session header is the tracked thread, harness prefixes excluded and replayed parent turns cut at the declared ordinal, since the `user_message` event this plan once cited does not exist on 0.153; OpenCode: a non-synthetic user message with no parent session and a matching session directory; Hermes: an active user row with no cron or compaction prefix, no parent session, no delegate marker; OpenClaw: provenance absent, no transport or sender id, no blocked-notice marker, partitioned by session key; Cursor: the user-query wrapper in the blob store with no subagent info on the chat);
- two-leg reconciliation: every hook-attested prompt appears in the transcript and vice versa; request and message identifiers line up where the harness issues them, and where they do not (on Codex the two tool identifier spaces are *mostly* disjoint — `exec`-family calls are `exec-<uuid>` on leg 2 and `call_<id>` on leg 1, while the `collaboration*` tools share `call_` ids across both legs — and one transcript call can be several hook pairs; corrected 2026-09-04 against `spikes/codex-cli/fixtures/`, where the earlier "never match" is false, and stated in full at PRD V8) the join goes through turn identity and ordering;
- normalisation before any byte comparison: exactly one trailing newline stripped, the connector's byte-exact field used, and a legitimate difference between what was typed and what the model saw never treated as tamper evidence on its own;
- `keep going` check: every human message after the first equals the literal phrase;
- gap classification: clean versus dirty, with the exact appended lines for review;
- session continuity: fresh start versus resume, second root agent in the workspace, second harness in the workspace, traced invocations of known harness binaries;
- config-deviation attribution: matched to a traced tool call or not;
- version drift, model identity and changes, subagent lineage;
- seal-time workspace attribution: every sealed file explained by a traced edit, write, or shell command;
- redaction-log-aware hash verification.

**Conformance suite**: recorded fixtures per harness (goal, `keep going`, subagent, tool call, large tool output, compaction, stop and resume, model switch) fed through the parser; assertions on prompt recovery, human-message classification, tool pairing, lineage, identity, and reconciliation. First-party and community connectors pass the same suite.

### 4.3 Capture legs per harness (as measured in M0)

Tiers are the measured outcome of the nine spikes and are recorded as a decision in D23. `elevated_review` means an organizer reads the trace before the entry is accepted, rather than an automatic seal; it is not a failing grade.

| Harness | Leg 1 (transcript) | Leg 2 (hooks) | Launch pins | Cursor | Partition key | Tier |
|---|---|---|---|---|---|---|
| Claude Code CLI | project JSONL + subagents + tool-results, plus raw API bodies (`OTEL_LOG_RAW_API_BODIES=file:`) | project `command` `SessionStart` shim plus `--settings` http hooks incl. `UserPromptSubmit`, `ConfigChange`, `SubagentStart/Stop`, `PreCompact`, `PostModelSwitch` | telemetry env, `--settings`, `--session-id`, `CLAUDE_CONFIG_DIR`; pin at or above 2.1.259 | inode + byte offset | none needed (process-scoped pins leaked nothing) | full |
| Codex CLI | `$CODEX_HOME/sessions/**/rollout-*.jsonl` | `.codex/hooks.json`, read at session start, trust managed or bypassed | dedicated `CODEX_HOME`, native binary launch and process-group kill | `ordinal` | first `session_meta.cwd` | full |
| OpenCode | `opencode.db` `event` table + `part` rows + capped tool-output spill | project `.opencode` plugin shim on `event`, `chat.system.transform`, `chat.messages.transform`; SSE bus as the resume witness | fixed port and password, `plugin.loaded` arming | `event.rowid` (global `seq` unsafe) | `session.directory` | full |
| Claude Desktop | project JSONL; raw bodies only under the app-wide exception | folder-scoped `.claude/settings.json` hooks, 16 of 17 events, no `SessionStart` ever | arm only | inode + byte offset | `metadata.user_id.session_id` on raw bodies | elevated_review |
| Codex desktop app | the participant's real `~/.codex` rollouts, filed by date | `.codex/hooks.json`, but only after CLI-granted trust; mandatory arming step | arm only, trust before the goal thread | `ordinal` | first `session_meta.cwd` | elevated_review |
| Hermes | `state.db` messages + `api_content` sidecar + `sessions.system_prompt` | plugin shim on the 23 hooks of 0.19.0, gated by an app-wide `plugins.enabled` key | dedicated `HERMES_HOME`, `HERMES_ENABLE_PROJECT_PLUGINS=1` | `messages.id` | `sessions.cwd` | elevated_review |
| OpenClaw | `openclaw-agent.sqlite` `transcript_events` plus the archive tables | plugin shim on `llm_input`, `before/after_agent_run`, `before/after_tool_call`; hot-disableable in 0.7 s | gateway mode, runtime pin per model | `(session_id, seq)` (rowid unsafe) | `ctx.sessionKey` | elevated_review |
| Cursor CLI | `store.db` blob tree (full tool results and system prompt); the JSONL is a display artefact | `.cursor/hooks.json`; `beforeSubmitPrompt`, `afterAgentResponse` and `stop` fire on a TTY only | three env pins, interactive CLI only, `-p` banned | `store.db` rowid | `meta.json` `cwd` | elevated_review |
| DeepSeek Harness | `session.jsonl` (compression off) + spill store | Cordis plugin shim on `session/event`, `llm/stream` | dedicated `DSH_HOME`, telemetry disabled | inode + byte offset | not established | not supported in v1 |

### 4.4 Start flow

1. `goalcup start --harness X` → preflight (grant, verified email, rules version, window open, approved build and manifest hash, harness version in range, upload connectivity, no conflicting entry).
2. Create empty workspace outside any Git worktree; write folder-scoped tracker config; record baseline manifest excluding those files. The tracker's artifact root lives elsewhere, never above the workspace.
3. CLI mode: launch the harness fresh with pins. Desktop mode: print the folder, wait for a new session with `cwd` = workspace and start source `startup`. Where the connector declares a human arming step, print it and wait: on the Codex desktop app that is `codex -C <workspace>`, `/hooks`, trust the twelve entries, quit, then open a new thread in the app.
4. Wait for the connector's arming signal for this workspace. Until it arrives the entry sits in `arming_pending` and no clock exists. A session opened before arming runs blind and silent for its whole life with nothing on screen to say so, which is the failure this step exists to prevent. Where the connector declares no start signal, as on Claude, the launch the daemon performed plus the first transcript record is the signal, and a missing `SessionStart` there is not an anomaly.
5. On the first submitted prompt: validate length (blocking hook where available; recorded as a violation where the harness has no rejection surface), send prompt hash and baseline to the backend, receive the signed receipt, bind the session, begin continuous upload.
6. If anything fails before the prompt is submitted, no entry exists and the participant may retry. Once the prompt is dispatched the run record is immutable.

### 4.5 Privacy boundary

- Only entry-bound sessions are captured. Declared shared harness stores may be read only to
  partition and discard foreign records before persistence. App-wide settings are modified only
  through the four declared exceptions in D21, each bracketed with before and after hashes and
  disclosed on the public entry.
- Most harnesses keep their transcripts in one home-wide or agent-wide store, so "observe only this folder" is enforced at capture time: the daemon reads the store, keeps the rows whose partition key matches the tracked workspace, and discards the rest before anything is hashed or shipped. It reads the day's Codex session directory and the home-wide SQLite stores of OpenCode, Hermes and OpenClaw to do that, and `RULES.md` says so plainly rather than implying the tracker only ever opens files inside the workspace.
- The daemon watches other harness stores during a run solely to detect a second harness in a bound workspace; nothing outside bound workspaces is persisted.
- Redaction patterns are public; the redaction log makes every altered span auditable.
- Raw traces remain private; the local buffer is encrypted per run; acknowledged chunks are deleted per the documented cleanup policy.

### 4.6 Trust posture

Tamper-evident, not tamper-proof. Signed allowlisted releases, per-run credentials, hash-chained events, frequent acknowledgements, immutable uploads, stored tracker and manifest versions, two-leg reconciliation, dirty-gap review. A modified tracker binary is ineligible. Kernel monitoring and hardware attestation are out of scope.

### 4.7 Tracker work tracker — deleted 2026-09-08

This section was twenty-four checkboxes with one ticked, and it stayed that way while nearly all of it shipped. A reader opening the only document in the tree shaped like a status board concluded the build did not exist. `STATUS.md` is the one work tracker — what is in flight, what landed, and what is knowingly deferred — and the boxes here were not ticked in passing, because two work trackers is how this happened.

## 5. Workstream B — Auth and event backend (deltas)

- Open source. Keys, production data, and deployment config stay private.
- Adds the acknowledgement service: countersigned chain checkpoints per entry, stored immutably; this is the witness that makes local chains evidence.
- Accepts the redaction log and config-deviation events as first-class records.
- Runs the verifier crate on every sealed entry and on demand during the run for organizer views.
- Everything else from v0.1 section 5 stands.

## 6. Workstream C — website and race board (deltas)

- Install and preflight instructions per harness and mode, including the desktop "open this folder" step.
- Per-run cards show capture tier, health states from section 3, and "back online" recovery events.
- Public entry shows capture tier and the disclosed tracker config exception.
- Everything else from v0.1 section 6 stands.

## 7. Workstream D — compliance, research, judging (deltas)

- Deterministic checks are the verifier's output; the compliance queue is built on it.
- Review workflow has three standard cases: dirty gaps (read appended lines), config deviations (attributed or not), unexplained sealed files.
- Research dataset is ATIF plus raw, de-identified per the retention policy.
- Everything else from v0.1 section 7 stands.

## 8. Shared contracts

As v0.1, plus: **connector manifest**, **redaction log**, **config-deviation event**, **acknowledgement checkpoint**, and the **ATIF mapping** per harness.

## 9. Cross-system acceptance scenarios (additions)

- A participant starts an entry on Claude Code CLI, and a separate everyday Claude Code session in another folder produces no daemon event and no upload.
- A Claude Desktop entry binds on the first prompt of a new conversation opened in the workspace folder; a conversation opened elsewhere is ignored; a second one in the same folder is flagged.
- The daemon is killed while the agent keeps working; on restart it catches up, reports "back online", and the verifier classifies the gap as dirty and lists the appended lines.
- The agent edits `.claude/settings.json`; the daemon restores it within a second and the verifier attributes the edit to the tool call.
- A `codex exec` invoked from a Claude Code Bash tool call is flagged as a second harness.
- `goalcup-verify` on a participant's own trace directory produces the same verdict as the backend.
- An over-length first prompt is blocked by the hook on Claude Code and Codex and never creates an entry.
- A participant opens a Codex desktop thread before granting hook trust; the daemon stays in `arming_pending`, refuses to stamp the clock, and prints the arming recipe instead of starting a blind run.
- A parallel Codex session in another folder writes into the same home-wide store on the same day, and none of its rows reaches the chain.
- A subagent completes on Claude Code and its notification arrives through the prompt hook; the verifier does not count it as a human message.

## 10. Known limitations to state honestly

- Extended thinking is not fully captured on Claude Code.
- Capture is bounded by the declared evidence surfaces. Tool-output bytes are not complete:
  Codex truncates in band with no spill file, OpenCode's spill file is itself capped, and
  OpenClaw persists a fraction of a large output with the full bytes surviving only in a hook
  payload.
- Auxiliary model calls are invisible to the transcript on four harnesses out of four measured (title generators, follow-up suggestion agents), so "every API call accounted for" is a claim about the conversation, not about the account.
- Cursor and desktop attach entries have thinner or single-leg evidence and are labeled accordingly. On three harnesses part of the record exists in only one leg, so both legs are mandatory there rather than merely corroborating.
- Formats are unstable; a mid-run harness update may require parser work after the event, and at least one harness updates itself during a run whatever the rules say.
- A human contribution that never touches the workspace or the agent's context is undetectable by design and is also not a violation.

## 11. Open decisions

1. Exact T0, timezone, judging dates.
2. Tested version ranges per connector at T-3 days.
3. Retention, deletion, organizer access roles.
4. Storage quotas for raw bodies and spill files over 168 hours.
5. Git providers monitored during the judging lock.
6. Judging rubric, shortlist threshold, sponsor awards.

## 12. Build order

Phase 0: protocol, manifest schema, threat model, mock server, conformance-suite skeleton.
Phase 1: daemon core plus the three `full`-tier connectors (Claude Code CLI, then Codex CLI and OpenCode) plus their verifier parsers, end to end against the mock server, `goalcup-verify` producing the EXPECTED verdict on the committed M0 fixtures of all three. *(Corrected 2026-09-08: this said "passing". The three M0 fixture sets are deliberately non-compliant scripts and must Fail with exit 2; a run that turned any of them green would mean the rule had been broken, not the milestone reached. PRD section 7 states it at length, and says so precisely because a future executor reading "passing" would try to fix the fixtures.)*
Phase 2: minimal production backend with acknowledgement service.
Phase 3: the `elevated_review` connectors, shims, desktop attach and the Codex app arming recipe, signed releases, contributor guide published.
Phase 4: participant experience.
Phase 5: compliance queue, publication verification, rehearsal.
