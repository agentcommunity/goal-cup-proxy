# Capturing a complete autonomous-run record from the outside — Codex and Cursor

> **Pre-spike research, superseded.** Written 2026-09-02 from documentation and source reading, before the M0 capture spikes. `docs/CAPTURE-RESEARCH.md` is the measurement and `DECISIONS.md` D23 is the tier table; read `docs/CAPTURE-RESEARCH.md` section 8 — the errata for these files — before trusting anything here.

Research date: 2026-09-02. Codex CLI latest: 0.152.0 (Sep 1 2026). Cursor CLI latest changelog entry: Aug 26 2026. Codex docs have moved to `learn.chatgpt.com/docs/*`; Cursor docs to `cursor.com/docs/*`.

Confidence key: **H** = primary source read directly; **M** = official doc summarized or single strong secondary; **L** = inference / secondary only.

---

## Part A — OpenAI Codex

### A1. Session / rollout files on disk

**Location & naming (H).** `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread_uuid>[_<rollout_id>].jsonl`; a "revert keeps the thread ID stable while creating a new immutable rollout file" (one thread can span several files). Each line is `{"timestamp": ..., "ordinal"?: u64, <flattened RolloutItem: "type", "payload">}`. Writes are one JSON object + `\n`, `write_all` then `flush()` per line via a bounded async channel; no explicit fsync. — https://raw.githubusercontent.com/openai/codex/main/codex-rs/rollout/src/recorder.rs

**Line types (M-H).** `session_meta` (id, timestamp, cwd, originator, cli_version, instructions, source, model_provider, git sha/branch/origin), `turn_context` (turn_id, cwd, approval_policy, sandbox_policy, **model, effort, summary**, collaboration_mode, truncation_policy — repeated every turn), `response_item`, `event_msg`, `compacted` (with `replacement_history`), plus newer `world_state` and `inter_agent_communication_metadata`.

**What is persisted (H, from `codex-rs/rollout/src/policy.rs`).**
- `response_item` persisted: Message, AgentMessage, Reasoning, LocalShellCall, FunctionCall, ToolSearchCall, **FunctionCallOutput**, ToolSearchOutput, CustomToolCall, **CustomToolCallOutput**, WebSearchCall, ImageGenerationCall, Compaction, ContextCompaction.
- `event_msg` always persisted: TokenCount, ThreadGoalUpdated, ThreadRolledBack, TurnAborted, TurnStarted, TurnComplete, ThreadSettingsApplied.
- `event_msg` persisted only when `ThreadHistoryMode::Legacy`: UserMessage, AgentMessage, AgentReasoning, AgentReasoningRawContent, PatchApplyEnd, ContextCompacted, McpToolCallEnd, WebSearchEnd, ImageGenerationEnd. **`Legacy` is `#[default]`**. In Paginated mode TurnItems are stored inside `ItemCompleted` events instead. Detect Paginated by checking for `item_completed` lines.
- **Limited vs Extended persistence (H):** `codex exec` starts threads with `persist_extended_history: false` while the TUI passes `true`; in Limited mode `PatchApplyEnd`, `ExecCommandEnd`, `McpToolCallEnd`, `Error`, `WebSearchEnd` are dropped. — https://github.com/openai/codex/issues/18533

**Tool outputs: full or truncated? (H)** The rollout stores the *model-facing* output. Shell output is bounded before it reaches history (10 KiB / 256 lines head+tail); since PR #20260 (Apr 30 2026) MCP results are byte-truncated in rollouts. The rollout is exactly "every token the agent saw"; it is *not* the raw stdout of every command. — https://github.com/openai/codex/pull/20260

**Reasoning (H):** `response_item` type `reasoning` (encrypted content + summaries) is persisted; `AgentReasoning`/`AgentReasoningRawContent` event_msgs are persisted in Legacy mode.

**Token counts / model (H):** `event_msg.token_count` carries `info.total_token_usage`, `info.last_token_usage`, `model_context_window`, `rate_limits`; model and reasoning effort are in every `turn_context`.

**Subagents / multi-agent (M):** `SessionSource` includes `SubAgent(SubAgentSource)`; sub-agent threads are separate rollout files, linked by `inter_agent_communication_metadata` lines. Hooks `SubagentStop` give `agent_transcript_path`.

**Compression / indexes (H):** Background job compresses "cold" rollouts to `.jsonl.zst` only when ≥ 7 days old; active files stay plain JSONL. Side indexes: `~/.codex/session_index.jsonl` and `state_5.sqlite` (metadata, not content).

**`~/.codex/history.jsonl` (M):** one record per typed prompt, `{session_id, ts, text}`; `[history] persistence = "none"` disables it but does **not** stop rollouts.

### A2. Hooks

**Status (H):** 12 events, enabled by default (`[features] hooks = false` to disable), non-managed hooks require a trust review keyed to the hook definition's hash; managed hooks via `requirements.toml` bypass trust; config is **loaded at session start** ("configuration changes take effect on the next session"). Discovery: `~/.codex/hooks.json`, `~/.codex/config.toml [hooks]`, `<repo>/.codex/hooks.json`, `<repo>/.codex/config.toml`, plugin `hooks/hooks.json`, `requirements.toml`. — https://learn.chatgpt.com/docs/hooks

**Surfaces (M):** "Hooks are supported across the ChatGPT desktop app, Codex CLI, and IDE extension"; GA May 2026. Codex cloud: not listed.

**Payload (H):** common: `session_id`, `cwd`, `hook_event_name`, `model`, `transcript_path` (nullable; "not a stable interface"), `permission_mode`, `turn_id`. Per event: `SessionStart.source` (startup/resume/clear/compact); `PreToolUse` `tool_name, tool_use_id, tool_input`; `PostToolUse` + `tool_response` (model-facing output); `PermissionRequest`; `UserPromptSubmit.prompt`; `SubagentStart/Stop` (`agent_id, agent_type, agent_transcript_path, last_assistant_message`); `Stop` (`stop_hook_active, last_assistant_message`); `Pre/PostCompact.trigger`. 0.152.0 adds `Interrupt`. Not exposed: reasoning, per-call token usage.

**`notify` (H):** `notify = ["cmd"]` receives JSON `{type:"agent-turn-complete", "thread-id", "turn-id", cwd, "input-messages", "last-assistant-message"}`; only that one event.

### A3. OpenTelemetry export

`[otel] exporter = none|otlp-http|otlp-grpc`, `trace_exporter`, `metrics_exporter`, `log_user_prompt` (default false → prompt redacted), per-exporter `endpoint/headers/protocol/tls`. Log events: `codex.conversation_starts` (model, reasoning, sandbox/approval), `codex.api_request`, `codex.sse_event` (token counts), `codex.websocket_request/event`, `codex.user_prompt`, `codex.tool_decision`, `codex.tool_result`. — https://learn.chatgpt.com/docs/config-file/config-reference

`codex.tool_result` (H): fields `tool_result_seq, tool_name, tool_namespace, call_id, duration_ms, success, output_truncated, arguments, output (preview), mcp_server`; output preview is cut at `max_bytes`. **Agent message text is not an OTel event** → OTel alone is not a complete transcript. Config is read at startup; needs a local OTLP receiver.

### A4. Headless / programmatic

**`codex exec --json` (H):** JSONL: `thread.started{thread_id}`, `turn.started`, `turn.completed{usage}`, `turn.failed`, `item.started/updated/completed`, `error`. Items: `agent_message{text}`, `reasoning{text}` (summary only), `command_execution{command, aggregated_output, exit_code, status}`, `file_change{changes[path,kind], status}` (no diff body), `mcp_tool_call{server, tool, arguments, result, error}`, `web_search`, `todo_list`. `codex exec resume [--last|<id>]`; `--ephemeral` skips the rollout entirely; exec runs in Limited persistence. — https://learn.chatgpt.com/docs/non-interactive-mode

**`codex app-server` (H):** JSON-RPC over stdio / `--listen ws://` / `unix://`; `thread/start` (`ephemeral: true` = no rollout), `thread/resume`, `turn/start`, `turn/steer`, `turn/interrupt`; notifications `item/started|completed`, `item/agentMessage/delta`, `item/reasoning/*`, `item/commandExecution/outputDelta`, `turn/completed`, `thread/tokenUsage/updated`. "The Codex VS Code extension and the Codex desktop application both leverage app-server"; threads persist as rollouts. `codex mcp-server` is deprecated in favor of app-server. A tracker *could* be the app-server client (owns all I/O), but that is a custom harness, not the participant's normal Codex. — https://learn.chatgpt.com/docs/app-server

### A5. Model identity

Per turn in `turn_context.model` / `effort` / `summary` (H); `ThreadSettingsApplied` event_msg is always persisted, so `/model`, `/effort`, fast-mode switches leave a record; also in `session_meta`, OTel `codex.conversation_starts`, hooks common `model`. `codex exec --json` does **not** carry model.

### A6. Human- vs harness-authored messages (H/M)

Order at session start in the rollout: `response_item role:"developer"` (base instructions, permissions), `response_item role:"user"` containing `# AGENTS.md instructions` wrapped in `<INSTRUCTIONS>`, `response_item role:"user"` with `<environment_context>`, `response_item role:"user"` collaboration-mode note, `turn_context`, and **the genuine human prompt as `event_msg` `{type:"user_message", payload:{message, images, local_images, audio, text_elements}}`** followed by its `response_item role:"user"`. Compaction summaries appear as `compacted` lines. Tool results are `function_call_output`/`custom_tool_call_output`. Rule for auditors: **a human input = an `event_msg/user_message` line** (Legacy default) — including queued messages typed mid-turn and app-server `turn/steer`. Cross-check with `UserPromptSubmit` hook `prompt`, `notify.input-messages`, and `history.jsonl`. Harness-injected user-role `response_item`s (AGENTS.md, env context, hook `additionalContext`, sub-agent `send_input`) have no `user_message` twin. — https://github.com/openai/codex/discussions/12668

### A7. Existing tools (all read `~/.codex/sessions`; none capture beyond the rollout)

- codex-trace (PixelPaw-Labs) — viewer, live SSE tail, collaboration chains, MIT, active. https://github.com/PixelPaw-Labs/codex-trace
- langfuse/codex-observability-plugin — `Stop` hook reads `transcript_path`, replays turns/tools/subagents/usage into Langfuse; Codex ≥ 0.128. https://github.com/langfuse/codex-observability-plugin
- codex-transcript-viewer, codex-session-export, codex-transcripts, codex-replay, codex-session-viewer, agent-sessions (macOS, multi-agent incl. Cursor Agent).
- OTel collector recipes: Last9, Coralogix, Monad.

### A8. Launch modes vs local observability

| Mode | Rollout on disk | Hooks | OTel | Verdict |
|---|---|---|---|---|
| CLI TUI | Yes, Extended persistence | Yes | Yes | **Complete** (H) |
| `codex exec` | Yes unless `--ephemeral`; Limited persistence | Yes | Yes | Complete for model-facing content (H) |
| IDE extension | Yes — app-server client, rollouts in `$CODEX_HOME/sessions` | Yes (GA May 2026) | M | Complete (M) |
| Codex desktop app | Yes — writes `sessions/…/rollout-*.jsonl` into shared `CODEX_HOME` | Yes | M | Complete (M); app can also dispatch **cloud** tasks, which are not |
| Codex cloud | No local file | — | — | **Unobservable locally** (H) |

### A9. Tamper surface

- Rollouts are plain, user-writable JSONL with no checksum, signature, or sequence MAC; `ordinal` is optional. `codex delete` permanently removes; `codex archive` moves.
- Escape hatches: `--ephemeral`, `thread/start{ephemeral:true}`, `CODEX_HOME` pointed elsewhere, `--ignore-user-config`, `-c` overrides, `[features] hooks=false`, `otel.exporter="none"`.
- Hooks and OTel config are read at session start, so a participant can't silently disable them mid-session without a restart — but `codex resume` is a new session start (`SessionStart.source="resume"`), which is exactly what "keep going" after a crash looks like.
- **Tailing + per-line hashing is sound** given: (a) the recorder only appends and flushes per line; (b) compression happens only after 7 days; (c) a revert creates a *new* file (`_<rollout_id>` suffix) — watch the directory, not one file. Hash chain each line as it appears and ship it off-host immediately. What hashing cannot prove: that the participant didn't run a second, untracked Codex (different `CODEX_HOME`).

---

## Part B — Cursor

### B1. Transcripts on disk (all undocumented formats)

**IDE agent (M-H):**
- `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb` (SQLite, table `cursorDiskKV`): `composerData:<id>`, `bubbleId:<composerId>:<bubbleId>` (per message: `text`, `type` 1=user/2=assistant, `richText`, `toolFormerData` with `name/params/userDecision/result`, `allThinkingBlocks`, `inputTokens/outputTokens`, `thinkingDurationMs`, `checkpointId`). Tool results are stored here in full. File is routinely 1–3 GB. — https://github.com/Callum-Ward/cursaves/blob/main/docs/how-cursor-stores-chats.md ; https://vibe-replay.com/blog/cursor-local-storage/
- `~/.cursor/projects/<path-derived-slug>/agent-transcripts/<session>/<session>.jsonl` (+ subagent JSONLs). Cursor staff (Apr 2026): contains user messages, assistant text and **tool-call inputs only — tool outputs intentionally excluded**; thinking may appear as `[REDACTED]`; advice: "log them yourself" with a `postToolUse` hook. No per-row timestamps, tokens, model, or stable IDs (logged, "no implementation timeline"). This is the file `transcript_path` points at. — https://forum.cursor.com/t/accessing-the-full-agent-transcript-in-cursor/157311 ; https://forum.cursor.com/t/richer-agent-transcripts-lifecycle-data-for-observability-langfuse-stop-hooks/166592

**CLI (`agent`, formerly `cursor-agent`) (M):** `~/.cursor/chats/<workspace-hash>/<session-uuid>/store.db` (SQLite `blobs`, `meta` with `agentId, name, mode, createdAt, lastUsedModel`) + `meta.json`; ACP-mode sessions in `~/.cursor/acp-sessions/`. Headless (`-p`) runs "write Claude Code-compatible JSONL" transcripts (Feb 2026 changelog) — location undocumented.

**Completeness verdict:** neither store is documented; the JSONL is lossy (no tool outputs); state.vscdb is richest but is a live SQLite DB.

### B2. Hooks (the real capture surface) (H — from cursor.com/docs/hooks)

Config: `~/.cursor/hooks.json` (user), `<project>/.cursor/hooks.json`, Team (Enterprise dashboard), Enterprise MDM; precedence Enterprise > Team > Project > User. "Cursor watches hooks config files and reloads them automatically." CLI also reads and merges Claude Code `settings.json` hooks (Jan 2026 changelog).

Common input: `conversation_id`, `generation_id` (changes per user message), `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`, `model`, `model_id`, `model_params[{id,value}]`, `transcript_path` (null if transcripts disabled).

Events and content-bearing fields:
- `beforeSubmitPrompt`: `prompt`, `attachments[{type,file_path}]` — **the only place the human-typed text is exposed**
- `afterAgentResponse`: `text` (final assistant text); `afterAgentThought`: `text` (aggregated thinking), `duration_ms`
- `preToolUse`/`postToolUse`/`postToolUseFailure`: `tool_name, tool_input, tool_use_id, cwd`, `tool_output` ("JSON-stringified result payload"), `duration`, `error_message`
- `beforeShellExecution`/`afterShellExecution`: `command, cwd, sandbox`, `output` ("Full output captured from the terminal"), `duration`
- `beforeMCPExecution`/`afterMCPExecution`: `tool_name, tool_input, mcp_server_name, url|command`, `result_json`
- `beforeReadFile`: `file_path, content, attachments`; `afterFileEdit`: `file_path, edits[{old_string,new_string}]`
- `sessionStart` (`session_id, is_background_agent, composer_mode`), `sessionEnd`, `subagentStart/Stop` (`subagent_model`, `agent_transcript_path`, `tool_call_count`…), `preCompact` (observational), `stop` (`status, loop_count`; may return `followup_message` → auto-resubmit, `loop_limit` default 5).
- IDE-only: `beforeTabFileRead`, `afterTabFileEdit`, `workspaceOpen`.

Environment matrix (docs): every agent hook fires in IDE **and CLI**; Cloud Agents run only command hooks from the repo's `.cursor/hooks.json`, skip `sessionStart/End`, `beforeMCPExecution`. `afterAgentThought/afterAgentResponse` in CLI since May 7 2026. Exit 0 = allow, exit 2 = block; default fail-open unless `failClosed: true`. Hooks do **not** expose: the system prompt/rules Cursor injects, per-call token usage. — https://cursor.com/docs/hooks

### B3. Telemetry / admin surfaces

No OTel exporter. Admin API (Teams/Enterprise): `POST /teams/filtered-usage-events` returns `timestamp, userEmail, model, kind, tokenUsage, conversationId, cloudAgentId, isHeadless` — **metadata only, no content**. Useful as an independent model-identity witness if a team plan is available. — https://cursor.com/docs/account/teams/admin-api

### B4. Headless / programmatic

**`agent -p --output-format stream-json` (H):** `system/init{session_id, model, cwd, permissionMode}`, `user{message}`, `assistant{message}` (`--stream-partial-output` adds deltas with `timestamp_ms, model_call_id`), `tool_call{subtype:started|completed, call_id, tool_call{args…, result…}}` ("includes both input arguments and full results"), `result{is_error, duration_ms, result, session_id, request_id, usage}`. **Thinking is suppressed in print mode.** `--resume`, `--continue`, `agent ls`, `--model`, `--force/--yolo`. — https://cursor.com/docs/cli/reference/output-format

**Cloud Agents API v1 (H):** `POST /v1/agents`, `POST /v1/agents/{id}/runs`, `GET /v1/agents/{id}/runs/{runId}/stream` SSE with `status, assistant, thinking, tool_call{callId,name,status,args,result,truncated}, result, done`. Runs live server-side; a local tracker can only mirror the SSE stream. — https://cursor.com/docs/cloud-agent/api/overview

### B5. Model identity

Per hook invocation: `model`, `model_id`, `model_params` — the most granular source; records mid-run switches at the next hook. stream-json: `init.model` only. Transcript JSONL: none per row. store.db: `lastUsedModel`. Cloud API: `model.id`+`params` per run.

### B6. Human vs harness

Human text surfaces exactly once: `beforeSubmitPrompt.prompt` (with `attachments`, which lists rules/files Cursor will inject). Each human submission increments `generation_id`; a `stop`-hook `followup_message` re-submission also produces a new generation but is attributable to the hook script. Rules/system prompt, `beforeReadFile.content`, and MCP results are harness-authored and appear only in their own hook events. Transcript-JSONL `user` rows do not distinguish typed text from injected context — do not rely on them.

### B7. Existing tools

- Hook-based: Entire (docs.entire.io/agents/cursor), hamzafer/cursor-hooks (audit.sh dumps every payload), Agentic Control Plane / Git AI / Enkrypt guardrail hooks, Langfuse replay via `stop`+transcript.
- DB/transcript readers: SpecStory (`specstory watch` reading `~/.cursor/chats/`), cursor-chronicle (read-only state.vscdb incl. `toolFormerData` results), cursaves, dwqs/cursor-trace, agent-sessions, agentgrep.
- None provides tamper evidence or a unified IDE+CLI+cloud record.

### B8. Launch modes vs local observability

| Mode | Best local source | Gaps | Verdict |
|---|---|---|---|
| IDE agent | Hooks (prompt, thinking, assistant text, tool in/out, shell output, model per event) + state.vscdb backup | injected rules/system prompt; per-call tokens | **Near-complete via hooks** (H) |
| CLI interactive | Same hooks + `~/.cursor/chats` store.db | same | Near-complete (M) |
| CLI headless `-p` | stream-json (everything except thinking) + hooks (thinking via `afterAgentThought`) | none material | **Complete** (H) |
| Cloud/background agent (`&`, web, API, Slack) | None locally. Repo `.cursor/hooks.json` can POST from the VM — partial, self-reported | sessionStart/End, MCP-before, read-only phase | **Unobservable by a local tracker** (H) |

### B9. Tamper surface

- `hooks.json` **hot-reloads**: the participant (or the agent itself, via `afterFileEdit` on `.cursor/hooks.json` in the repo) can remove the logging hook mid-run with no restart. Only Enterprise Team/MDM hooks are non-overridable. Mitigation: log `hook_event_name` sequence numbers per `conversation_id` and treat any gap/absence of `afterAgentResponse` between `beforeSubmitPrompt`s as a break; have the hook script refuse to run if the repo copy differs from a pinned hash.
- state.vscdb and transcript JSONL are user-writable, no integrity. `transcript_path` may be null if transcripts are disabled.
- Tailing+hashing the JSONL is sound mechanically but the file is lossy, so hash the **hook stream** instead.

---

## Recommended capture design

**Codex (confidence H):**
1. Tracker starts *before* the run, watches `$CODEX_HOME/sessions/**`, tails every new `rollout-*.jsonl`, hash-chains each line, and streams line+hash off-host in real time. This alone is the complete model-facing record. Also tail sub-agent rollouts and `inter_agent_communication_metadata`.
2. Install an organizer-supplied `~/.codex/hooks.json` (participant trusts it once at start): `UserPromptSubmit`, `SessionStart` (source = startup/resume), `Stop`, `SubagentStart/Stop`, `Pre/PostCompact`, each POSTing to the tracker. Independent witness for "human typed X".
3. Optionally set `[otel] exporter="otlp-http"` to a local receiver with `log_user_prompt=true` for a third witness.
4. Proof rule: human-authored content = `event_msg/user_message` lines (Legacy mode; detect `item_completed` if Paginated) ∪ `UserPromptSubmit.prompt`. Allowed set: initial prompt, and "keep going". Anything else, or a `SessionStart.source="startup"` with a new `session_meta.id`, is a violation.
5. Require the TUI or IDE/app (Extended persistence); if `codex exec` is allowed, forbid `--ephemeral`.

**Cursor (confidence M-H):**
1. Ship an organizer `~/.cursor/hooks.json` (and enforce identical repo copy) with `beforeSubmitPrompt`, `afterAgentThought`, `afterAgentResponse`, `postToolUse`, `postToolUseFailure`, `afterShellExecution`, `afterMCPExecution`, `afterFileEdit`, `beforeReadFile`, `sessionStart/End`, `subagentStart/Stop`, `preCompact`, `stop`; the script forwards the raw stdin JSON plus a monotonic counter per `conversation_id`, hash-chained, to the tracker. Only source with prompt, thinking, assistant text, tool outputs and model together.
2. Prefer headless `agent -p --output-format stream-json` for the run; capture stdout and let hooks add thinking.
3. Copy `transcript_path` files and `~/.cursor/chats/**` at the end as corroboration, not as the record.
4. Proof rule: human content = `beforeSubmitPrompt.prompt`; count `generation_id`s; a `stop.followup_message` from the organizer script is the sanctioned "keep going".
5. Detect hook tampering: expected event cadence; a `beforeSubmitPrompt` with no subsequent `afterAgentResponse`, or a change in the hooks.json hash reported by `sessionStart`, disqualifies.

## Unobservable modes (state these in the rules)

- **Codex cloud tasks** (from web, GitHub, Slack, Linear, or dispatched from the desktop app/CLI): no local rollout, hooks or OTel. (H)
- **Cursor Cloud/Background Agents** (`&` handoff, web, Slack, API): no local files; repo hooks fire only in the writable phase and are self-reported from the VM. (H)
- **Codex `--ephemeral` / app-server `ephemeral:true`** and **Cursor with transcripts disabled + hooks removed**: nothing durable is written; forbid by rule and detect by absence of expected events. (H)
- **Partial:** Codex `exec` Limited persistence and Cursor headless stream-json (no thinking) — acceptable with compensating hook sources. (H)
