# Capturing a complete record of an autonomous Claude Code run from the outside (no proxy)

> **Pre-spike research, superseded.** Written 2026-09-02 from documentation and source reading, before the M0 capture spikes. `docs/CAPTURE-RESEARCH.md` is the measurement and `DECISIONS.md` D23 is the tier table; read `docs/CAPTURE-RESEARCH.md` section 8 — the errata for these files — before trusting anything here.

Research date: 2026-09-02. Scope: Claude Code v2.1.258 native binary (Mach-O arm64) on macOS, subscription OAuth login. Findings marked **[verified]** were checked on this machine (test run: `claude -p` with hooks + raw-body telemetry, session id `11111111-2222-4333-8444-555555555555`). Everything else is cited to docs or primary sources.

---

## 1. Session transcript files on disk

**Path.** `~/.claude/projects/<project>/<session-id>.jsonl`, where `<project>` is the cwd path with non-alphanumerics replaced by `-` (truncated to 200 chars + hash if longer). Overridable with `CLAUDE_CONFIG_DIR` and (v2.1.234+) `CLAUDE_CODE_PROJECT_DIR_NAME`. Anthropic states the entry format "is internal to Claude Code and changes between versions, so scripts that parse these files directly can break on any release." — https://code.claude.com/docs/en/sessions#where-transcripts-are-stored

**Record types [verified, 230k records across 27 local projects, v2.1.181–2.1.258]:** `assistant`, `user`, `attachment`, `system`, `queue-operation`, `last-prompt`, `custom-title`, `ai-title`, `mode`, `permission-mode`, `pr-link`, `started`, `result`, `bridge-session`, `atis-latch`, `relocated`, `worktree-state`, `frame-link`, `file-history-snapshot`, `file-history-delta`, `agent-name`, `cost-state`, plus artifact-related types.

**Common envelope [verified]:** `uuid`, `parentUuid` (chain; `logicalParentUuid` on compaction boundaries), `isSidechain`, `sessionId`, `timestamp` (ISO), `cwd`, `gitBranch`, `version`, `entrypoint` (`cli` | `claude-desktop`), `userType`, `promptId`, `permissionMode`.

**`user` records [verified]:** `message.{role,content}` where content is a plain string (typed prompt) or a block array (`text`, `tool_result`, `image`); `isMeta`, `origin`, `promptSource` (`typed` | `sdk` | `system`), `toolUseResult` (structured tool output, e.g. Bash `{stdout,stderr,interrupted,isImage,noOutputExpected,persistedOutputPath,persistedOutputSize}`), `sourceToolUseID`, `sourceToolAssistantUUID`, `toolDenialKind`, `isCompactSummary`, `isVisibleInTranscriptOnly`.

**`assistant` records [verified]:** `message.{model,id,type,role,content[],stop_reason,stop_sequence,usage}` — content blocks are `thinking` (with `signature`), `text`, `tool_use`; `usage` includes cache read/creation tokens, `service_tier`, `speed`; plus `requestId` (`req_…`) and `attributionSkill/Plugin/McpServer/McpTool`. Model is on every assistant record.

**`system` subtypes [verified]:** `stop_hook_summary`, `api_error`, `compact_boundary`, `turn_duration`, `local_command`, `away_summary`, `model_refusal_fallback`, `informational`.

**`attachment` subtypes [verified]:** `hook_success` (hook name, event, full stdout), `hook_additional_context`, `prompt_snapshot` (**contains the `systemPrompt` array** — present in v2.1.258 transcripts), `model` (`identity.modelId`, marketing name, knowledge cutoff), `environment`, `skill_listing`, `deferred_tools_delta`, `agent_listing_delta`, `mcp_instructions_delta`, `total_tokens_reminder`, `task_reminder`, `file`, `edited_text_file`, `queued_command`, `command_permissions`, `compact_file_reference`, `date`/`date_change`, `session_context`, `instructions`, `nested_memory`, `remote_session_change`, `read_truncation_notice`, `invoked_skills`, `goal_status`, `auto_mode`.

**Subagents [verified + docs].** `~/.claude/projects/<project>/<session-id>/subagents/agent-<agentId>.jsonl` plus `agent-<agentId>.meta.json` (`{agentType, description, toolUseId, spawnDepth}`). Records carry `isSidechain: true` and `agentId`. Docs: "Each transcript is stored as `agent-{agentId}.jsonl`"; deleted with the parent after `cleanupPeriodDays`. — https://code.claude.com/docs/en/sub-agents ; https://code.claude.com/docs/en/claude-directory#cleaned-up-automatically

**Tool result content [verified].** Stored inline in full up to a size threshold (largest inline observed: 43 KB). Larger outputs are spilled to `<session-id>/tool-results/<id>.txt` and the transcript stores a `<persisted-output>` block with a 2 KB preview and the file path; `toolUseResult.persistedOutputPath` points to the spill file. Important: the preview *is* what the model saw, so the transcript remains faithful to model input; the spill file is extra data the model did not see. — https://code.claude.com/docs/en/claude-directory#cleaned-up-automatically

**Compaction [verified].** Appends (does not rewrite): a `system/compact_boundary` record (`compactMetadata: {trigger: manual|auto, preTokens, durationMs, preCompactDiscoveredTools, preservedSegment{headUuid,anchorUuid,tailUuid}}`, `logicalParentUuid`) followed by a `user` record with `isCompactSummary: true`, `isVisibleInTranscriptOnly: true`. Pre-compaction history stays in the file: "the original messages stay in the session transcript" — https://code.claude.com/docs/en/checkpointing. Subagent auto-compaction uses the same marker (issue: https://github.com/anthropics/claude-code/issues/16944).

**Resume / continue.** Same file is appended: "If you resume the same session in two terminals without forking, messages from both interleave into one transcript." `--fork-session` / `/branch` copies to a new session id. SessionStart hooks re-run on resume with `source: "resume"`; mid-session hook outputs are replayed from the transcript, not re-run. — https://code.claude.com/docs/en/sessions#branch-a-session ; https://code.claude.com/docs/en/hooks

**Append-only?** Mostly, but not guaranteed. Writes are asynchronous ("The transcript file is written asynchronously and may lag the in-memory conversation" — https://code.claude.com/docs/en/hooks#common-input-fields). Timestamps are not monotonic in file order (27/968 records out of order in one local file) [verified]. Docs describe `<session>.jsonl.superseded-<timestamp>` and `<session>.orphaned-…jsonl` files: "A previous transcript for the session that Claude Code set aside instead of overwriting or deleting it" — i.e. file-replacement events exist. Concurrent writers can corrupt the file (non-atomic appends): https://github.com/anthropics/claude-code/issues/20992. Retention sweep deletes transcripts after `cleanupPeriodDays` (default 30, min 1): https://code.claude.com/docs/en/claude-directory#cleaned-up-automatically ; https://github.com/anthropics/claude-code/issues/59248. `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` disables writing entirely: https://code.claude.com/docs/en/env-vars.

**Is it complete enough to reconstruct every token sent to the model? No — semantically close, byte-wise no [verified by diffing against raw API bodies].** Differences observed in the test run:
- The first API request's `messages[0]` was a `<system-reminder>` text block (user email/context) that has **no counterpart** in the JSONL user record (which stores only the typed prompt string).
- SessionStart hook context is sent to the API as a `system`-role message; the JSONL stores it as an `attachment/hook_additional_context`.
- Tool definitions (13 tools with full JSON schemas in the request) are **not** in the JSONL (only names via `deferred_tools_delta`).
- The system prompt is in `attachment/prompt_snapshot` (new in recent versions; older transcripts lack it), but the request also carries a billing-header system block and cache_control markers absent from the JSONL.
- CLAUDE.md / rules content reaches the model inside system-reminders; the JSONL has `instructions`/`nested_memory` attachments only in some versions — coverage not guaranteed.
- Thinking: the JSONL keeps whatever the API returned. In one Opus 4.8 file all 172 thinking blocks were empty strings (summarized/adaptive thinking returned nothing); another file had 24–2788 chars per block [verified]. Raw API bodies redact thinking entirely (below).

---

## 2. Hooks

**Events (current reference):** `SessionStart`, `Setup`, `UserPromptSubmit`, `UserPromptExpansion`, `PreToolUse`, `PermissionRequest`, `PermissionDenied`, `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`, `Notification`, `MessageDisplay`, `SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`, `Stop`, `StopFailure`, `TeammateIdle`, `InstructionsLoaded`, `ConfigChange`, `CwdChanged`, `DirectoryAdded`, `FileChanged`, `WorktreeCreate`, `WorktreeRemove`, `PreCompact`, `PostCompact`, `PreModelSwitch`, `PostModelSwitch`, `Elicitation`, `ElicitationResult`, `SessionEnd`. Types: `command`, `http`, `mcp_tool`, `prompt`, `agent`. — https://code.claude.com/docs/en/hooks

**Common stdin fields:** `session_id`, `prompt_id` (matches OTel `prompt.id`), `transcript_path`, `cwd`, `permission_mode`, `effort.level`, `hook_event_name`; inside subagents also `agent_id`, `agent_type`. [verified: also an undocumented `scratchpad_dir`.] Per event [verified]: `UserPromptSubmit.prompt` (exact typed text); `PreToolUse.{tool_name,tool_input,tool_use_id}`; `PostToolUse.{tool_response,duration_ms}` (full structured output); `Stop.{stop_hook_active,last_assistant_message,background_tasks,session_crons}`; `SessionStart.{source, model?}`; `SessionEnd.reason`. Docs add `SubagentStop.agent_transcript_path`, `PreCompact.{trigger,custom_instructions}`, `PostToolUseFailure`, `ConfigChange.{source,file_path}`, `PreModelSwitch/PostModelSwitch.{from_model,to_model}`.

**Subagents:** "Hooks from settings files, managed policy settings, and plugins also run inside subagents… the input carries the `agent_id` and `agent_type`." A `Stop` hook in subagent frontmatter becomes `SubagentStop`. — https://code.claude.com/docs/en/hooks

**Model in hooks:** "Only `SessionStart` hooks can receive a `model` field, and Claude Code doesn't always include it." Use `PostModelSwitch.to_model` to follow changes. "There is no `$CLAUDE_MODEL` environment variable." [verified: SessionStart payload had no `model` field.]

**Configuration and mid-run edits.** Hooks live in `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`, managed settings, plugin `hooks/hooks.json`, skill/subagent frontmatter, and `--settings <file-or-json>`. **Hooks are not snapshotted at startup**: "Claude Code watches your settings files and reloads them when they change… including edits to `permissions`, `hooks`" (https://code.claude.com/docs/en/settings#when-edits-take-effect) and "Direct edits to hooks in settings files are normally picked up automatically by the file watcher" (https://code.claude.com/docs/en/hooks). Each detected change fires `ConfigChange` (blockable except `policy_settings`). A participant can therefore add/remove hooks mid-run; a tracker must log `ConfigChange` and treat hook silence as an anomaly. Precedence matters: `--settings` overrides project/local; `disableAllHooks` is read after precedence, so a launcher-supplied `--settings` with `disableAllHooks:false` defeats a project-level `true`; only managed settings beat `--settings`. — https://code.claude.com/docs/en/hooks#disable-or-remove-hooks ; https://code.claude.com/docs/en/settings-reference (`allowManagedHooksOnly`)

**Hook outputs in transcript [verified]:** hook stdout and injected context are persisted as `attachment/hook_success` and `attachment/hook_additional_context`, and the model receives them as a `system`-role message (raw body). Cloud sessions do not read local `~/.claude/settings.json`.

---

## 3. OpenTelemetry export

Enable with `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_METRICS_EXPORTER`/`OTEL_LOGS_EXPORTER` (`otlp|console|none`), `OTEL_EXPORTER_OTLP_{PROTOCOL,ENDPOINT,HEADERS}`, `OTEL_LOGS_EXPORT_INTERVAL` (default 5000 ms). Content gates (all off by default): `OTEL_LOG_USER_PROMPTS`, `OTEL_LOG_ASSISTANT_RESPONSES` (v2.1.193+), `OTEL_LOG_TOOL_DETAILS`, `OTEL_LOG_TOOL_CONTENT` (needs beta tracing), `OTEL_LOG_RAW_API_BODIES`; `CLAUDE_CODE_OTEL_CONTENT_MAX_LENGTH` (default 60 KB). — https://code.claude.com/docs/en/monitoring-usage

**Events:** `user_prompt` (`prompt_length`, `prompt` redacted by default, `command_name`, `message.uuid`), `assistant_response` (`response`, `model`, `request_id`), `api_request` (`model`, `cost_usd`, `duration_ms`, `input/output/cache_read/cache_creation_tokens`, `request_id`, `client_request_id`, `query_source` = `main|subagent|auxiliary`, `effort`, `agent.name`, `skill.name`, …), `api_error`, `api_refusal`, `tool_result` (`tool_name`, `tool_use_id`, `success`, `duration_ms`, `decision_source`, `tool_input` ≤4 KB with TOOL_DETAILS), `tool_decision` (`decision`, `source` = `config|hook|user_permanent|user_temporary|user_abort|user_reject`), `permission_mode_changed`, `auth`, `mcp_server_connection`, `api_request_body`, `api_response_body`. Standard attributes: `session.id`, `app.version`, `app.entrypoint`, `user.account_uuid`, `user.email` (OAuth), `prompt.id`, `message.uuid`, `client_request_id`.

**Full content is available only via `OTEL_LOG_RAW_API_BODIES` (added v2.1.111; changelog: https://code.claude.com/docs/en/changelog).** Doc text: "Bodies include the entire conversation history… The bodies contain the full conversation history, including the system prompt, every prior user and assistant turn, and tool results… Claude Code always redacts Claude's extended-thinking content from these bodies." `=1` emits inline bodies truncated at 60 KB; `=file:<dir>` "writes untruncated bodies to `.request.json` and `.response.json` files under that directory, and the events carry a `body_ref` path." Must be set "in your shell, user settings, or managed settings. It's ignored in project and local settings." — https://code.claude.com/docs/en/monitoring-usage

**[verified]** With `OTEL_LOG_RAW_API_BODIES=file:<dir>` the native binary under OAuth wrote `<uuid>.request.json` (118 KB each; keys `model, messages, system, tools, betas, metadata, max_tokens, thinking, context_management, output_config, diagnostics, stream`; 4 system blocks incl. billing header; 13 tool schemas; `metadata.user_id` embeds `session_id`) and `<req_id>.response.json` (`model, id, content, stop_reason, usage`). One file per API attempt. Response bodies keep `text`/`tool_use` blocks; thinking is redacted per docs. File-mode writing did not depend on the exporter.

**Endpoint independence:** OTLP endpoint is separate from the model API endpoint; no `ANTHROPIC_BASE_URL` change needed. `OTEL_*` variables are stripped from subprocesses (Bash tool, hooks, MCP). Managed settings can pin OTLP endpoints and strip developer overrides. Tracing (beta) adds `claude_code.llm_request` spans.

---

## 4. Other surfaces

**Headless / SDK CLI [verified].** `claude -p --output-format stream-json --verbose [--include-partial-messages] [--forward-subagent-text] --session-id <uuid>` emits: `system/init` (`model`, `tools`, `mcp_servers`, `permissionMode`, `apiKeySource`, `claude_code_version`, `agents`, `skills`, `slash_commands`, `plugins`, `capabilities`, `session_id`), `system/hook_started|hook_response`, `assistant` (`message.model`, `usage`, `uuid`, `parent_tool_use_id`), `user` (full `tool_result` block + `tool_use_result`), `system/task_summary`, `system/post_turn_summary`, `system/api_retry`, `system/compact_boundary`, `rate_limit_event`, `result` (`modelUsage` per model with `canonicalModel`, `total_cost_usd`, `num_turns`, `permission_denials`). Subagent messages carry `parent_tool_use_id`; `--forward-subagent-text` (v2.1.211+) adds text and thinking, nested to any depth (v2.1.219+). `--session-id` "must be a valid UUID" [verified pins the id]. `--input-format stream-json` + `--replay-user-messages` allow the tracker to own stdin. `--bare` skips hooks/CLAUDE.md but "doesn't use your subscription login" (API key only). — https://code.claude.com/docs/en/headless ; https://code.claude.com/docs/en/cli-reference ; https://code.claude.com/docs/en/agent-sdk/typescript

**Desktop app [verified locally].** Writes to the same `~/.claude/projects` with `entrypoint: "claude-desktop"`; human-typed messages show `promptSource: "sdk"` with `origin: {kind: "human"}` (the app drives Claude Code through the SDK), so `promptSource=sdk` alone does not mean automation.

**Remote Control.** Session runs locally; local JSONL still written (`bridge-session` records and `remote_session_change` attachments observed). Phone/browser messages arrive as ordinary user turns. Disabled when `ANTHROPIC_BASE_URL` ≠ `api.anthropic.com` (v2.1.196+). — https://code.claude.com/docs/en/remote-control

**Channels.** Inbound messages are delivered to the model as `<channel source="plugin:…">` events and carry `origin: {kind: "channel", server}`; only allowlisted senders pass. Observable locally via JSONL/hooks. — https://code.claude.com/docs/en/channels

**Claude Code on the web.** Runs in Anthropic-managed VMs; no local transcript, no local hooks/OTel; `--teleport` copies history down afterwards. Not observable by a local tracker. — https://code.claude.com/docs/en/claude-code-on-the-web

---

## 5. Existing tools

| Tool | Approach | Captures | Misses / status |
|---|---|---|---|
| claude-trace (badlogic/lemmy) | `--require` fetch interceptor in Node | Full request/response incl. system prompt, tools, thinking | Broken on native binary (issue #48, open since 2025-10-22). PR #56 adds a reverse proxy via `ANTHROPIC_BASE_URL` — unmerged. Any working variant is now a proxy. https://github.com/badlogic/lemmy/issues/48 |
| hanqunfeng/claude-trace fork | Local reverse proxy via `ANTHROPIC_BASE_URL` | Same | Proxy |
| cc-viewer (weiesky) | Intercepts API requests when it launches `claude` | Request/response, sub-agent tagging | Proxy-style launcher; https://github.com/weiesky/cc-viewer |
| claude-code-log (daaain) | Parses `~/.claude/projects` JSONL → HTML/MD, watch mode | Messages, tool use/results, thinking, sidechains | Viewer only; https://github.com/daaain/claude-code-log |
| claude-devtools, claude-code-trace (delexw), Claudoscope | JSONL viewers; delexw live-tails; Claudoscope uses FSEvents | Context attribution, subagents, tokens | Viewers only |
| ccusage | Reads JSONL `usage` fields | Tokens/cost | No content; https://github.com/ryoppippi/ccusage |
| claude-code-otel (ColeMurray) | OTLP collector → Prometheus/Loki/Grafana | Metrics + events | Metadata-centric; https://github.com/ColeMurray/claude-code-otel |
| claude_telemetry "claudia" (TechNickAI) | Python Agent SDK wrapper → OTel | Prompts, tool I/O, tokens | Requires SDK (API-key path) |
| Langfuse official integration | `Stop` hook reads `transcript_path` incrementally | Turns, generations, tool spans | No system prompt; https://langfuse.com/integrations/developer-tools/claude-code |
| pdhoolia/langfuse-claude-code-plugin | 9 hooks incl. SubagentStop/PreCompact/PostCompact | Thinking, subagent transcripts, compaction | Small project |
| disler/claude-code-hooks-multi-agent-observability | All hook events → HTTP → SQLite | Hook payloads, subagent lifecycle | Demo-grade |

Nothing existing combines raw-body telemetry + hooks + hashed JSONL tailing for tamper evidence.

---

## 6. Human-authored vs harness-authored messages

Signals available on `user` records:
- **`origin.kind`** (official enum): `human`, `channel` (+`server`), `peer` (+`from`, `senderTaskId`, `body`), `task-notification`, `coordinator`, `auto-continuation`, `unclassified`. "Claude Code treats a user message with no `origin` as unattributed." — https://code.claude.com/docs/en/agent-sdk/typescript (SDKMessageOrigin). [verified locally: absent on 93% of user records — all tool results and older-version prompts.]
- **`promptSource`**: `typed` | `sdk` | `system` [verified; undocumented].
- **`isMeta: true`** = harness-injected text: skill bodies, `<local-command-caveat>` slash-command output, "Continue from where you left off.", image placeholders [verified].
- **`isCompactSummary: true` + `isVisibleInTranscriptOnly: true`** = compaction summary [verified].
- **Tool results**: content array with `tool_result` blocks, `toolUseResult`, `sourceToolUseID` [verified].
- **Slash-command expansion**: `system/local_command` records and `isMeta` caveats; `UserPromptExpansion` hook fires.
- **Hook outputs / reminders**: `attachment` records (never `user`); system-reminders are injected at API-request time and are visible only in raw request bodies.
- **Interrupts**: user text `[Request interrupted by user]`.

**Is a typed message unambiguously identifiable?** From the JSONL alone: *high confidence but not proof*. Rule: `type=user` AND content is a string or text-only blocks AND `isMeta≠true` AND `isCompactSummary≠true` AND (`origin.kind=human` OR `promptSource=typed`/`sdk`). Corroborate with the `UserPromptSubmit` hook (fires only for submitted prompts, delivers exact `prompt` text) and with raw request bodies (a new human turn appears as a new trailing `user` message that is not a `tool_result`). Three independent sources agreeing is the evidentiary standard; a human turn missing from any one of them is a red flag.

---

## 7. Model identity

- JSONL: `message.model` on every `assistant` record (main and subagent files) [verified]; `attachment/model.identity.modelId` at session start [verified].
- stream-json: `system/init.model`, `assistant.message.model`, `result.modelUsage[<model>].canonicalModel` [verified].
- OTel: `model` on `api_request`, `api_error`, `assistant_response`, `api_request_body`; `query_source`/`agent.name` distinguish subagents.
- Raw bodies: `request.model` and `response.model` [verified].
- Hooks: `SessionStart.model` (optional), `PostModelSwitch.to_model`.
- Mid-run `/model` changes are visible as differing `message.model` values within one file [verified].

---

## 8. Tamper surface

- Files are mode `0600`, owned by the participant [verified]. No checksums, signatures, or integrity metadata exist in the JSONL.
- Participant controls: edit/delete JSONL at will; `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` or `--no-session-persistence` to suppress writing; `CLAUDE_CONFIG_DIR` to write elsewhere; `disableAllHooks` / hook edits (hot-reloaded); unset `OTEL_*` in their shell; `cleanupPeriodDays: 1`; `/rewind`, `/branch`, `--fork-session`.
- Harness-side non-tamper events a hasher will see: async out-of-order appends [verified], occasional file replacement (`.superseded`/`.orphaned` files documented), corruption under concurrent writers.
- **Tailing + per-line hash chain is sound for post-hoc edit detection** if the chain is shipped off-box immediately. Requirements: watch by path *and* inode (flag replacement), also watch `<session>/subagents/` and `<session>/tool-results/`, tolerate partial trailing lines, and record `ConfigChange` hook payloads. It cannot detect what the harness never wrote, a killed tracker, or a second `claude` started under another `CLAUDE_CONFIG_DIR`. Cross-source consistency closes those gaps: every JSONL `assistant.requestId` must have a matching raw `<req_id>.response.json` and `<uuid>.request.json` whose trailing messages equal the JSONL chain; Anthropic-issued `req_…`/`msg_…` ids cannot be fabricated by the participant.

---

## 9. Base URL and OAuth

- `ANTHROPIC_BASE_URL` **does** work with subscription OAuth: "Setting only that variable, without a gateway credential, doesn't replace the subscription. Requests still route through the gateway, but a saved claude.ai login remains the active credential…" Side effects: Remote Control disabled (v2.1.196+), MCP tool search off by default. — https://code.claude.com/docs/en/llm-gateway#subscriptions-and-gateways
- Policy: "OAuth authentication is intended exclusively for purchasers of Claude Free, Pro, Max, Team, and Enterprise subscription plans and is designed to support ordinary use of Claude Code and other native Anthropic applications… developers may not collect, store, or intermediate Claude.ai credentials or session tokens…" Also: "The Claude Code binary must not be modified." — https://code.claude.com/docs/en/legal-and-compliance
- Net: a local logging proxy that forwards the OAuth bearer sits inside "intermediate… session tokens" and outside "ordinary use". Avoiding proxies is the conservative reading. Raw-body telemetry achieves the same capture using a documented first-party feature.

---

## Recommended capture design for Claude Code

**Launcher-owned environment (tracker starts `claude`; participant never sets env).**
1. **Primary, byte-exact model I/O:** `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_LOG_RAW_API_BODIES=file:<tracker-owned dir>`, `OTEL_LOGS_EXPORTER=otlp` → tracker's local OTLP receiver (also `OTEL_LOG_USER_PROMPTS=1`, `OTEL_LOG_TOOL_DETAILS=1`, `OTEL_LOG_ASSISTANT_RESPONSES=1`). Gives every request (system prompt, tools, full message history, model) and response (content, `usage`, `request_id`) per API attempt, subagents included. Set via shell env; project/local settings cannot disable it. Hash and ship each body file on creation. **Confidence: high** (verified on 2.1.258 + OAuth; documented since v2.1.111).
2. **Event skeleton and human-input attestation:** hooks supplied via `--settings '{"disableAllHooks":false,"hooks":{…}}'` (beats project/local), recording `SessionStart`, `UserPromptSubmit` (exact typed text — enforce equals "keep going"), `UserPromptExpansion`, `PreToolUse`/`PostToolUse`/`PostToolUseFailure`, `SubagentStart`/`SubagentStop`, `PreCompact`/`PostCompact`, `PreModelSwitch`/`PostModelSwitch`, `ConfigChange`, `PermissionRequest`, `Stop`, `SessionEnd`, `Notification`. Each hook POSTs its stdin (`type: http`) to the tracker. **Confidence: high** for payloads; **medium** for robustness (hooks hot-reload) — log `ConfigChange` and alert on hook silence.
3. **Tamper-evident local record:** FSEvents/inotify tail of `~/.claude/projects/<slug>/<session-id>.jsonl`, `<session-id>/subagents/*.jsonl`, `<session-id>/tool-results/*`; per-line SHA-256 hash chain, inode tracking, immediate off-box shipping. Provides `origin`/`isMeta`/`isCompactSummary`/`promptSource` provenance, thinking blocks (absent from raw bodies), `prompt_snapshot`, hook attachments. **Confidence: medium** (schema undocumented and version-volatile).
4. **Optional strongest tier (headless):** run `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --forward-subagent-text --session-id <uuid> --replay-user-messages`, with the tracker as the only stdin writer (UI exposes a single "keep going" button). Makes human input structurally impossible outside the tracker. **Confidence: high** for the flags; trade-off is loss of the interactive TUI.

**Cross-checks organizers should run on traces:** (a) every JSONL `assistant.requestId` ↔ raw `.response.json` id ↔ OTel `api_request.request_id`; (b) each raw request's trailing `user` message equals the JSONL chain and, for non-tool turns, has a matching `UserPromptSubmit` hook record with text "keep going"; (c) `session.id`/`prompt.id` continuity across hooks and OTel; (d) `system/compact_boundary` count = `PreCompact` hook count; (e) `ConfigChange` events reviewed manually.

**What remains unobservable or weak:**
- Extended thinking: redacted in raw bodies; JSONL keeps only what the API returned (often summarized or empty).
- Claude Code on the web / cloud sessions: nothing local; must be banned for the event.
- Remote Control: local capture works, but inputs from phone/web look like `origin.kind=human` turns; rely on hook attestation (they still fire `UserPromptSubmit`) or ban RC (`disableRemoteControl`).
- A participant who runs a second, untracked `claude` (different `CLAUDE_CONFIG_DIR`) against the same repo: detectable only indirectly (git commits/file changes not attributable to traced tool calls).
- Schema drift: JSONL and even hook fields change per release; pin the Claude Code version for the event and re-run the verification script against it.
