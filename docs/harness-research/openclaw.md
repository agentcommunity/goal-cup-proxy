# OpenClaw external run-capture research (as of 2026-09-02, repo HEAD `21e356c`, package version 2026.8.1; latest release v2026.8.2 published 2026-09-01)

> **Pre-spike research, superseded.** Written 2026-09-02 from documentation and source reading, before the M0 capture spikes. `docs/CAPTURE-RESEARCH.md` is the measurement and `DECISIONS.md` D23 is the tier table; read `docs/CAPTURE-RESEARCH.md` section 8 — the errata for these files — before trusting anything here.

Headline finding that changes the premise of the question: **since OpenClaw 2.0 (v2026.8.1) transcripts are no longer JSONL files. They live in a per-agent SQLite database.** JSONL is migration-only input / export output. Everything below is verified against docs.openclaw.ai and the `main` source tree.

---

## 1. On-disk session/transcript storage

**Paths (High)**
- Per-agent runtime store (session rows AND transcript events): `~/.openclaw/agents/<agentId>/agent/openclaw-agent.sqlite` — https://docs.openclaw.ai/reference/session-management-compaction ("On-disk locations"), https://docs.openclaw.ai/concepts/session, https://docs.openclaw.ai/reference/database-schemas
- Global control-plane DB: `~/.openclaw/state/openclaw.sqlite` (audit ledger, cron, plugin state, etc.) — https://docs.openclaw.ai/reference/database-schemas
- `~/.openclaw/agents/<agentId>/sessions/` now holds only legacy/archive artifacts (`*.jsonl.deleted.<ts>.zst`, `*.reset.*`); `sessions/sessions.json` is a legacy migration input. "Gateway startup does not import them"; "Runtime code must not write... transcript JSONL... anymore" — https://docs.openclaw.ai/reference/session-management-compaction, https://github.com/openclaw/openclaw/pull/78595 (merged 2026-05-13).
- WAL journaling is used for local stores (`src/infra/sqlite-wal.ts`) — https://github.com/openclaw/openclaw/blob/main/src/infra/sqlite-wal.ts. Practical consequence: an external reader must open the DB read-only and expect `-wal` sidecars.

**SQLite schema (High)** — https://github.com/openclaw/openclaw/blob/main/src/state/openclaw-agent-schema.sql
- `transcript_events(session_id, seq, event_json, created_at) PRIMARY KEY(session_id, seq)` — each row's `event_json` is one transcript entry (same shape as the old JSONL line).
- `session_nodes(session_key PRIMARY KEY, current_session_id, entry_json, created_via IN ('operator','spawn','channel','cron','talk','run','plugin','internal'), created_actor_type IN ('human','agent','system'), parent_session_key, spawned_by, fork_source_*…)` — session rows (formerly `sessions.json`).
- `session_windows(session_id PRIMARY KEY, session_key, previous_session_id, reason IN ('initial','reset','rollover','fork','rewind','switch','recovery','compaction'), channel, account_id, model_provider, model, agent_harness_id, …)` — one row per transcript generation.
- `trajectory_runtime_events(session_id, seq, run_id, event_json, created_at)` — runtime diagnostics ledger (see below).
- `session_transcript_archives(… archive_blob BLOB, archive_sha256 TEXT …)` — compressed archive of deleted/reset generations.
- `session_transcript_active_events`, `session_transcript_fts` (FTS5 of user/assistant text).

**Transcript entry schema (High)** — https://github.com/openclaw/openclaw/blob/main/src/agents/sessions/session-manager-types.ts, https://github.com/openclaw/openclaw/blob/main/src/agents/sessions/session-manager-codec.ts, https://docs.openclaw.ai/reference/session-management-compaction ("Transcript event structure")
- Header: `{type:"session", version, id, timestamp, cwd, parentSession?}`.
- Entries share `{id, parentId, timestamp}` (tree-structured; `parentId` links). Types: `message`, `thinking_level_change`, `model_change {provider, modelId}`, `compaction {summary, firstKeptEntryId, tokensBefore}`, `reset {reason: new|reset|idle|daily|cron-stale}`, `branch_summary`, `custom` (not in model context), `custom_message` (in model context, `display` flag), `label`, `session_info`.
- `message.message.role` ∈ `user | assistant | toolResult | custom | bashExecution`. `AssistantMessage` carries `api, provider, model, responseModel?, responseId?, usage, stopReason, content: (TextContent|ThinkingContent|ToolCall)[]` — https://github.com/openclaw/openclaw/blob/main/packages/llm-core/src/types.ts. `toolResult` carries `toolCallId, toolName, isError, content[]`.

**Completeness**
- Tool results: persisted as `toolResult` messages (High). Tool-result truncation exists for context (`tool-result-truncation.ts`) and `tool_result_persist`/`before_message_write` plugin hooks can rewrite before persistence — https://docs.openclaw.ai/plugins/hooks. Whether the on-disk copy is the untruncated one in every path: **Medium — not fully verified**.
- Thinking blocks: `ThinkingContent` is a legal content block in persisted assistant messages; thinking is stripped **on replay**, not necessarily on disk ("Historical assistant thinking/reasoning blocks are stripped before replay") — https://docs.openclaw.ai/reference/transcript-hygiene. Provider-dependent (Anthropic redacted_thinking etc.). Medium.
- System prompt: **not stored in `transcript_events`**. Bootstrap files (AGENTS.md, SOUL.md, IDENTITY.md, USER.md, MEMORY.md…) are injected into the system prompt's Project Context on the first turn — https://docs.openclaw.ai/concepts/agent. The compiled system prompt is available only via plugin hook `llm_input.systemPrompt` (High) and, bounded/redacted, in the trajectory runtime ledger (`finalPromptText`, capped by `TRAJECTORY_RUNTIME_FINAL_PROMPT_MAX_BYTES`, `systemPrompt` is a drop-first key when oversized) — https://github.com/openclaw/openclaw/blob/main/src/trajectory/runtime.ts. Medium.
- Append-only vs rewrite: docs say transcript events are "append-only, tree-structured"; compaction appends a `compaction` entry and does not delete rows; reset appends a `reset` entry; deletion moves the generation into `session_transcript_archives` and removes rows in one transaction — https://docs.openclaw.ai/reference/session-management-compaction. But: "The Gateway is the authority: it may rewrite or rehydrate entries as sessions run", there is a `transcript_rewrite_watermarks` table and `transcript-rewrite.ts` / `thinking-replay-repair.ts` in the runner (media rewrite, thinking-signature repair). So: mostly append-only, with bounded targeted rewrites (Medium).
- Session maintenance (`session.maintenance`: `pruneAfter 30d`, `maxDiskBytes 10gb`, `maxEntries 500`) can delete old rows/artifacts — https://docs.openclaw.ai/reference/session-management-compaction. For a 168h run this is a real risk if the participant's agent creates many sessions; set `mode: "warn"` or protect via pin/archive.

**Subagents (High)**
- Session key `agent:<agentId>:subagent:<uuid>` (depth 2: `…:subagent:<uuid>:subagent:<uuid>`); own transcript/generation in the same per-agent DB; linkage via `spawned_by` / `parent_session_key` columns on `session_nodes` and `spawnedBy`, `parentSessionKey`, `forkSource` on the session row — https://docs.openclaw.ai/tools/subagents, https://docs.openclaw.ai/gateway/protocol (sessions.create / sessions.patch notes), https://github.com/openclaw/openclaw/blob/main/src/config/sessions/types.ts. `fork` spawn context branches the parent transcript into the child. Completion is announced back as a user-role message with `provenance.kind = "inter_session"`, `sourceTool = "subagent_announce"` — https://github.com/openclaw/openclaw/blob/main/src/sessions/input-provenance.ts.
- Cron isolated runs: `cron:<jobId>`; boot hook runs: `agent:<id>:boot:<run-id>` — https://docs.openclaw.ai/automation/cron-jobs, https://docs.openclaw.ai/automation/hooks.

## 2. Hooks / plugins / webhooks

**Internal hooks (High)** — https://docs.openclaw.ai/automation/hooks
- Directory: `~/.openclaw/hooks/<name>/HOOK.md + handler.ts|js|index.ts|js`; also `<workspace>/hooks/` (opt-in) and `hooks.internal.load.extraDirs`. Config `hooks.internal.enabled/entries`.
- Events: `command:new|reset|stop`, `session:auto-reset|compact:before|compact:after|patch`, `agent:bootstrap` (mutable `context.bootstrapFiles`), `gateway:startup|shutdown|pre-restart`, `message:received|transcribed|preprocessed|sent`. Payload: `type, action, sessionKey, timestamp, context, messages[]`. These are observation points, "not a complete transport audit".
- "Hooks do not hot-reload; restart the Gateway after modifications." Bundled `command-logger` appends command events to a JSONL log.

**Plugin SDK hooks (High)** — https://docs.openclaw.ai/plugins/hooks, https://github.com/openclaw/openclaw/blob/main/src/plugins/hook-types.ts
- `llm_input` → `{runId, sessionId, provider, model, systemPrompt?, prompt, historyMessages[], imagesCount, tools?}` — full provider input including the compiled system prompt (emitted from `src/agents/cli-runner.ts` with `context.systemPrompt`).
- `llm_output` → `{runId, sessionId, provider, model, resolvedRef?, harnessId?, prompt?, assistantTexts[], lastAssistant?, usage, reasoningEffort?, fastMode?}`.
- `agent_end` → `{runId, messages[], success, error?, durationMs}`; `before_agent_run` gets prompt + session history + system prompt.
- `before_tool_call` → `{toolName, params, toolKind?, runId?, toolCallId?}`; `after_tool_call` → `{toolName, params, result?, error?, durationMs?}`.
- `message_received` → `{from, content, senderId?, threadId?, messageId?, sessionKey?, runId?, media?, metadata?}`; `message_sending`/`message_sent`.
- `subagent_spawned {childSessionKey, resolvedModel, resolvedProvider}`, `subagent_ended`, `session_start/end`, `before/after_compaction`, `model_call_started/ended` (metadata only, no content), `gateway_start/stop`.
- Context (`PluginHookAgentContext`): `runId, agentId, sessionKey, sessionId, modelProviderId, modelId, channel, accountId, chatId, senderId, trigger`.
- Gating: non-bundled plugins need `plugins.entries.<id>.hooks.allowConversationAccess: true` for `llm_input/llm_output/agent_end/before_agent_run/...`. Loading: `plugins.load.paths`, `plugins.allow/deny`, `plugins.entries.<id>.enabled`.

**Can a participant change this mid-run? (High — yes)** — https://docs.openclaw.ai/gateway/configuration ("Config hot reload")
- `~/.openclaw/openclaw.json` is watched; default mode `hybrid`. `hooks`, `plugins.entries.*`, `plugins.allow`, `plugins.deny`, `plugins.enabled`, `logging`, `agents`, `models` **hot-apply without restart**. `plugins.load`, `plugins.installs`, `gateway.*` require restart, which `hybrid` performs automatically. Plugin code edits need a restart, but a participant can disable a capture plugin by flipping `plugins.entries.<id>.enabled` live. Config writes are also possible over the WS API (`config.patch`, admin scope). Conclusion: an in-process plugin/hook is not tamper-resistant against the machine owner.

**Webhooks**
- `openclaw webhooks` CLI is only Gmail Pub/Sub setup/run — https://docs.openclaw.ai/cli/webhooks.
- The "webhooks" plugin is inbound TaskFlow control ("Neither operation starts an agent") — https://docs.openclaw.ai/plugins/webhooks.
- Gateway HTTP hooks `POST /hooks/agent` (starts an agent turn with `message`), `POST /hooks/wake`, `POST /hooks/<name>` mappings — an **injection surface** to monitor — https://docs.openclaw.ai/gateway/configuration-reference (Hook agent payload). No outbound "push every event" webhook exists (High).

## 3. Telemetry / OpenTelemetry

- Plugin `@openclaw/diagnostics-otel` (bundled in `extensions/diagnostics-otel`), config `diagnostics.otel.{enabled,endpoint,logs,sampleRate,captureContent,logsExporter}` — https://docs.openclaw.ai/gateway/opentelemetry, https://github.com/openclaw/openclaw/tree/main/extensions/diagnostics-otel
- Exports: metrics (tokens, cost, run duration, tool exec…), traces (model calls, harness lifecycle, tool execution, context assembly, tool loops), logs (OTLP or stdout JSONL). "Raw model/tool content is **not** exported by default." With `captureContent: true`: "bounded, redacted input messages, output messages, tool inputs, tool outputs, tool definitions… **System prompts remain excluded**" (docs), although the source has a `policy.systemPrompt` branch writing `gen_ai.system_instructions`/`openclaw.content.system_prompt` (`service-genai-content.ts`) — Medium on whether any config enables it. Thinking/reasoning parts are replaced with `{type:"reasoning", redacted:true}` — https://github.com/openclaw/openclaw/blob/main/extensions/diagnostics-otel/src/service-genai-content.ts. Session keys are replaced with `unknown` on low-cardinality attributes. Verdict: useful corroboration, **not** a complete record (High).
- Unrelated: "usage telemetry" is only a daily update check — https://docs.openclaw.ai/gateway/telemetry.

## 4. Headless / server modes

**Gateway WS protocol (High)** — https://docs.openclaw.ai/gateway/protocol, https://docs.openclaw.ai/concepts/architecture
- `ws://127.0.0.1:18789`, JSON frames `req/res/event`, first frame `connect` (challenge nonce, `role: operator`, scopes `operator.read|write|admin|approvals…`, device identity + pairing; loopback backend clients may omit device with the shared token).
- Subscriptions: `sessions.subscribe` (index changes), `sessions.messages.subscribe {sessionKey, includeApprovals?}` → `session.message` / `session.operation` / `session.tool` / `session.approval` events; broadcast `agent` events (streams `lifecycle|tool|assistant|usage|error|item|plan|approval|command_output|patch|compaction|thinking`, payload `{runId, seq, stream, ts, data, sessionKey?, sessionId?, agentId?}`) — https://github.com/openclaw/openclaw/blob/main/src/infra/agent-events.ts. `assistant`/`thinking` data = `{text, delta}`; `tool` data = `{phase: start|update|result, name, toolCallId, args (sanitized), result…}` — https://github.com/openclaw/openclaw/blob/main/src/agents/embedded-agent-subscribe.handlers.tools.start.ts. Content frames need `operator.read`.
- Lossiness: `chat.history` and `session.message` payloads are **display-normalized** (`projectSessionMessagePayload` → `projectChatDisplayMessage`; directive tags, tool-call XML stripped, oversized rows placeholdered) — https://github.com/openclaw/openclaw/blob/main/src/gateway/session-transcript-message.ts. Tool args in `agent` events pass through `sanitizeToolArgs`. So WS streaming is a good live tap but not byte-exact (High).
- `chat.inject` (admin, deprecated `<=2026.7`) appends an **assistant**-role transcript message without a model run — https://github.com/openclaw/openclaw/blob/main/src/gateway/server-methods/chat.ts. `chat.send`/`sessions.send`/`/hooks/agent` start turns. All are injection surfaces.

**CLI**
- `openclaw agent --message … --json` (through Gateway) emits an envelope `{ok, status, final, payloads, usage, costUsd, model, provider, sessionId, …}` — no per-step trace. `--local` runs the embedded agent against the same state dir (requires exclusive lock, so cannot coexist with a running Gateway). `openclaw agent exec` uses a **temporary state directory deleted afterwards** unless `--state-dir` — https://docs.openclaw.ai/cli/agent.
- `openclaw sessions --json`, `sessions tail --follow` (redacted: "prompt text, tool arguments, and tool result bodies are not printed"), `sessions export-trajectory --session-key … --output <dir>` (bundle: `events.jsonl`, `session-branch.json`, `metadata.json`, `prompts.json`, `artifacts.json`; redacted) — https://docs.openclaw.ai/cli/sessions, https://github.com/openclaw/openclaw/blob/main/src/trajectory/export.ts. There is no raw `sessions export`. `openclaw transcripts` is for **meeting** transcripts, unrelated — https://docs.openclaw.ai/cli/transcripts.
- `openclaw tui` (Gateway-backed, or `--local` embedded) and the browser Control UI both speak the Gateway WS — https://docs.openclaw.ai/cli/tui, https://docs.openclaw.ai/web/control-ui.
- Audit ledger (`openclaw audit`, `logging.audit.*`): metadata-only (`agent_run`, `tool_action`, inbound/outbound message with HMAC pseudonyms), never content; execution identity off by default — https://docs.openclaw.ai/gateway/audit, https://docs.openclaw.ai/cli/audit.

## 5. Model identity

- `model_change {provider, modelId}` transcript entries (appended via `appendModelChange`) — https://github.com/openclaw/openclaw/blob/main/src/agents/sessions/session-manager-entries.ts; `SessionContext.model` is derived from them.
- Every persisted assistant message carries `provider`, `model`, `responseModel?`, `api`, `usage` — https://github.com/openclaw/openclaw/blob/main/packages/llm-core/src/types.ts. This is the strongest per-turn evidence (High).
- `session_windows.model_provider/model/agent_harness_id`, session row `providerOverride/modelOverride` — https://docs.openclaw.ai/reference/session-management-compaction.
- Subagents: `subagent_spawned` hook gives `resolvedModel/resolvedProvider`; child transcript's own assistant messages carry model. `/model` switch → `model_change` entry + `session:patch` internal hook. `llm_output.resolvedRef` distinguishes `openai/gpt-5.4` vs `codex/gpt-5.4`.

## 6. Human vs harness-authored user messages

- Persisted user messages may carry `provenance: {kind: "external_user" | "inter_session" | "internal_system", originSessionId?, sourceSessionKey?, sourceChannel?, sourceTool?}`. Inter-session text is also prefixed `[Inter-session message]` in model input. Sources: `subagent_announce`, `main_session_restart_recovery`, `exec_approval_followup`, `agent_harness_task`, etc. — https://github.com/openclaw/openclaw/blob/main/src/sessions/input-provenance.ts (High).
- Channel messages get `__openclaw` metadata: `senderId, senderName, senderUsername, senderIdentity {type:"observation", pluginId, accountId, senderKind: human|bot|unknown}, senderIsOwner, replyToId, transport {channel, conversationRef, messageId, threadId}, media` — https://github.com/openclaw/openclaw/blob/main/src/sessions/user-turn-transcript.metadata.ts (High).
- Heartbeats: the prompt is "sent **verbatim** as the scheduled user message" (default text documented), run in `agent:<id>:main` or an isolated session; heartbeat/cron do not update `lastInteractionAt` — https://docs.openclaw.ai/gateway/heartbeat. Cron isolated runs live in `cron:<jobId>`. `context.assembled` trajectory events and `PluginHookAgentContext.trigger` carry `trigger` (`user|cron|heartbeat|…`).
- Session-level: `session_nodes.created_via` and `created_actor_type (human|agent|system)`.
- **Gap (Medium):** I did not verify that a plain `openclaw agent --message` / Control UI / TUI message is stamped with `provenance.kind="external_user"` (provenance is only attached when supplied; absence ≠ human). Treat "no provenance + no channel transport" as "local operator input" and corroborate with the `trigger` field from hooks/trajectory.

## 7. Third-party tools

- `rufuscodes/openclaw-session-viewer` and `VACInc/openclaw-tool-call-viewer` read `~/.openclaw/agents/main/sessions/*.jsonl` — **obsolete for 2.0** — https://github.com/rufuscodes/openclaw-session-viewer, https://github.com/VACInc/openclaw-tool-call-viewer
- `billmetangmo/langfuse-openclaw-exporter` relays session log files by read offset — file-based, likely obsolete — https://github.com/billmetangmo/langfuse-openclaw-exporter
- `MCKRUZ/openclaw-langfuse` plugin uses `before_agent_start` + `agent_end`, truncates input to 2k/output 4k chars, no tool calls — https://github.com/MCKRUZ/openclaw-langfuse
- `aliyun/openclaw-exporter-to-langfuse` (OTel GenAI spans) — https://github.com/aliyun/openclaw-exporter-to-langfuse; `ericmey/openclaw-otel` fork; Grafana dashboard 25068; SigNoz/LangWatch guides. None found that dumps the SQLite transcript verbatim (Medium — searched, not exhaustive).

## 8. Launch modes vs observability

All of these run through one Gateway and write the same per-agent SQLite: gateway daemon, `openclaw agent` (non-local), TUI, Control UI/WebChat, macOS app (owns a local launchd Gateway or connects remotely), iOS/Android nodes (WS `role: node`, read-only transcript cache), messaging channels, Docker (state volume mounted at `/home/node/.openclaw`) — https://docs.openclaw.ai/concepts/architecture, https://docs.openclaw.ai/platforms/macos, https://docs.openclaw.ai/platforms/ios, https://docs.openclaw.ai/install/docker. Observable via the DB and WS.

Not observable via a running Gateway, but still writing the same DB: `openclaw agent --local`, `openclaw tui --local` (exclusive lock on the state dir; Gateway must be stopped) — https://docs.openclaw.ai/cli/agent, https://docs.openclaw.ai/cli/tui.

## 9. Tamper surface

- SQLite files are plain, world-readable to the user, editable with any SQLite client; no per-row hashing, no signature, no hash chain on `transcript_events` (the only SHA-256 is on deleted/reset archive blobs and `openclaw backup sqlite` manifests) — https://github.com/openclaw/openclaw/blob/main/src/state/openclaw-agent-schema.sql, https://docs.openclaw.ai/cli/backup (High).
- The Gateway holds the DB open (WAL) and is the authority; it can rewrite entries (`transcript_rewrite_watermarks`, media/thinking repair). External edits while the Gateway runs are undefined behaviour but not detected.
- Incognito threads (Control UI) keep transcripts **in memory only** — https://docs.openclaw.ai/concepts/session. `openclaw agent exec` deletes its state dir. `openclaw sessions delete`, maintenance pruning, `resetArchiveRetention` remove data.
- Config hot-reload lets the owner disable hooks/plugins/OTel live (§2).

---

## Recommended capture design

1. **Primary record: continuous snapshot of `~/.openclaw/agents/*/agent/openclaw-agent.sqlite`** (High). Open read-only (`?mode=ro`, honour WAL), poll `transcript_events` by `(session_id, seq)` and `trajectory_runtime_events`, plus `session_nodes`/`session_windows`. Append each new row to a tracker-owned, hash-chained JSONL (`sha256(prev_hash || event_json)`) and periodically anchor the chain hash off-box (e.g. signed timestamp to organizers). This is the only source with per-turn `provider/model/usage`, full tool results, `model_change`, provenance and sender metadata. Run the tracker as a separate OS user/service the participant cannot stop without it being visible (tracker heartbeat to organizers).
2. **Corroborating live tap: Gateway WS operator client** (High for existence, Medium for fidelity): `connect` with `operator.read`, `sessions.subscribe`, `sessions.messages.subscribe` for every session key (including new subagent/cron keys from `sessions.changed`), and record `agent` events (`assistant`, `thinking`, `tool`, `lifecycle`). Detects sends (`chat.send`, `/hooks/agent`, channel messages) in real time and cross-checks DB rows.
3. **System-prompt capture: a tracker-owned plugin** registered via `plugins.load.paths` with `hooks.allowConversationAccess: true`, using `llm_input` (system prompt + history + tools), `llm_output`, `before_tool_call`/`after_tool_call`, `subagent_spawned`, `message_received`, writing to an append-only file the participant cannot modify. Note it hot-disables via config; log `config.get` `configRevisionHash` changes and treat plugin silence during an active run as a flag.
4. **Injection detection**: flag any user-role `transcript_events` row whose `__openclaw.transport`/`senderId` is present (channel input), any `provenance.kind != external_user` that is not `subagent_announce`/heartbeat, any `custom_message` entry, any `chat.inject`/`chat.send`/`sessions.send`/`/hooks/agent` observed on the WS or in the metadata-only audit ledger (`logging.audit.messages: all`), and any `session_windows` row with `hook_external_content_source` set. Enforce "keep going" by literal match on every human-attributed user message after the first.
5. **Lock down modes**: require gateway mode (forbid `agent --local`, `agent exec`, incognito); disable `session.maintenance` pruning (`mode: "warn"`, `maxDiskBytes: false`) or pin sessions; snapshot `openclaw.json` at start and on every `config.get` hash change; optionally enable `diagnostics-otel` with `captureContent: true` to a tracker collector as a third leg.

## Unobservable modes (High unless noted)
- Incognito Control UI threads (memory only).
- `openclaw agent exec` without `--state-dir` (temp state deleted).
- `agent --local` / `tui --local`: no Gateway → no WS tap, no Gateway-hosted plugins guaranteed (Medium: plugins do preload for `--local`), but DB rows still land.
- Codex/Claude-CLI harness runtimes (`agentRuntime.id: codex|claude-cli`): OpenClaw keeps only a transcript *mirror*; native compaction and tool records are harness-owned; `tool_result_persist` does not apply to Codex-native tool results — https://docs.openclaw.ai/plugins/codex-harness, https://docs.openclaw.ai/gateway/cli-backends. Restrict participants to the native `openclaw` runtime.
- Anything the model does outside the session (files written by tools) is not a "token" record; cover with git history.

## Unknowns / could not verify
- Exact per-path guarantee that tool results on disk are untruncated (Medium). The runner has `tool-result-truncation.ts`; hook `tool_result_persist` can rewrite.
- Whether `openclaw agent --message` / Control UI inputs are stamped `provenance.kind="external_user"` or left unstamped (Medium).
- Whether any `diagnostics.otel` setting actually enables system-prompt export (docs say excluded; code has a `policy.systemPrompt` path) (Low).
- `TRAJECTORY_RUNTIME_FINAL_PROMPT_MAX_BYTES` exact value and whether `finalPromptText` is recorded on every model call (Medium; constant exists in `src/trajectory/paths.ts`, capture cap 10 MiB per runtime).
- Precise `session.message` payload field list beyond `sessionKey, agentId, message, transcriptPosition, messageId, messageSeq, runId, sessionSnapshot` (Medium).
- Whether the v2026.8.2 release changed any of the above (release notes mention no storage/hook changes; source inspected is `main` at 2026-09-02).
