# Hermes Agent (Nous Research) — external run-capture research

> **Pre-spike research, superseded.** Written 2026-09-02 from documentation and source reading, before the M0 capture spikes. `docs/CAPTURE-RESEARCH.md` is the measurement and `DECISIONS.md` D23 is the tier table; read `docs/CAPTURE-RESEARCH.md` section 8 — the errata for these files — before trusting anything here.

Verified 2026-09-02 against the `main` branch of github.com/NousResearch/hermes-agent and hermes-agent.nousresearch.com/docs. Where a claim came from a source-file fetch, I note the file; where only the docs page was available, I note that. Confidence: **High** = read in official docs or source; **Medium** = official but partial/indirect; **Low** = inferred or third-party.

---

## 1. On-disk session/transcript storage

**Location / engine (High).** All sessions from every surface land in one SQLite DB, `~/.hermes/state.db` (WAL mode), overridable via `HERMES_HOME`. Per-session JSONL files are legacy; `~/.hermes/sessions/sessions.json` mirrors only gateway routing entries and is explicitly "not authoritative". Legacy snapshots: `~/.hermes/sessions/saved/*.json`.
- https://hermes-agent.nousresearch.com/docs/developer-guide/session-storage
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/sessions.md
- `DEFAULT_DB_PATH = get_hermes_home() / "state.db"` — https://raw.githubusercontent.com/NousResearch/hermes-agent/main/hermes_state.py

**Schema (High; `SCHEMA_VERSION = 27`, read from `hermes_state_common.py`).**
- `sessions(id, source, user_id, session_key, chat_id, chat_type, thread_id, display_name, origin_json, model, model_config, system_prompt, system_prompt_hash, parent_session_id, started_at, ended_at, end_reason, message_count, tool_call_count, input/output/cache_read/cache_write/reasoning_tokens, cwd, git_branch, git_repo_root, billing_provider, billing_base_url, billing_mode, estimated_cost_usd, actual_cost_usd, title, api_call_count, profile_name, rewind_count, archived, pinned, hidden, ...)`
- `messages(id AUTOINCREMENT, session_id, role, content, tool_call_id, tool_calls, tool_name, effect_disposition, timestamp, token_count, finish_reason, reasoning, reasoning_content, reasoning_details, codex_reasoning_items, codex_message_items, platform_message_id, observed, _compressed_summary, active, compacted, api_content, display_kind, display_metadata)`
- `system_prompts(hash PK, prompt)`; `session_model_usage(session_id, model, billing_provider, billing_base_url, billing_mode, task, api_call_count, tokens..., first_seen, last_seen)`; `async_delegations(...)`; `messages_fts` / `messages_fts_trigram` (FTS5); plus routing/lease/heartbeat tables.
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/hermes_state_common.py

**Completeness (High/Medium).**
- Full message history incl. tool calls and tool results, reasoning fields, and finish reason: High (schema above; session-storage doc).
- System prompt: stored per session as a snapshot (`sessions.system_prompt`, SHA-256 `system_prompt_hash`, deduped in `system_prompts`). High. Whether the stored snapshot is refreshed after compaction's "system prompt is rebuilt" step is **not verified**.
- `api_content` = "byte-fidelity sidecar that stores the exact content string sent to the API for a message when it differs from the regular content field (for ephemeral memory/plugin injections and persist overrides)". This is the key to "every token the model saw" for user turns. Medium (docs page text, not source-verified). https://hermes-agent.nousresearch.com/docs/developer-guide/session-storage
- MEMORY.md/USER.md are rendered into the system prompt as a frozen block at session start, so they are inside the stored `system_prompt`. High. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/memory.md
- SKILL.md content enters context via the `skill_view` tool, so it appears as a tool result in `messages`; the skills index is in the system prompt. High. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/skills.md
- **Not stored per message:** model/provider. The `messages` table has no model/provider/source column; attribution is per session (`sessions.model`) and per-model aggregate (`session_model_usage`). High.

**Append-only vs rewritten (High).** Messages are INSERTed; "every message and tool result is written to state.db as it happens". Compaction with the default `compression.in_place: true` "rewrites the live message list on the same session id" but old turns are "soft-archived under the same id (`active=0, compacted=1`)... never deleted". Rewind is "a soft delete in the SQLite session store (recoverable/inspectable), not a hard truncate". Summary messages carry the `[CONTEXT COMPACTION]` prefix and `_compressed_summary=1`. With `in_place: false`, each compaction creates a new session linked via `parent_session_id`. Hard DELETEs exist for `hermes sessions delete/prune` and cascade-deleting delegate children (`DELETE FROM messages WHERE session_id IN (...)` in hermes_state.py).
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/developer-guide/context-compression-and-caching.md
- https://github.com/NousResearch/hermes-agent/issues/21910 (rewind semantics)

**Subagents (High).** `delegate_task` children get their own `SessionDB` connection and session row; the child's session metadata carries `_delegate_from` (parent session id, stored inside `model_config` JSON, queried via `json_extract(model_config,'$._delegate_from')`) and `parent_session_id`. Children's full histories are persisted; the parent's transcript sees only the delegation call and the returned summary. Live per-child logs also land at `~/.hermes/cache/delegation/live/<delegation_id>/task-<n>.log` (+ `manifest.json`, pruned after 7 days).
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/tools/delegate_tool.py
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/hermes_state.py
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/delegation.md

**Format for export (High).** `hermes sessions export --format jsonl|md|qmd|html|trace` (trace = Claude Code JSONL for HF Agent Trace Viewer; `--upload` pushes to a private HF dataset; `--redact` default for trace). Default dir `~/.hermes/session-exports/`. There is a separate ShareGPT JSONL "trajectory" format (see §3). No ATIF anywhere in the repo/docs I read.
- https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/sessions.md

## 2. Hooks / plugin / event API

Four systems (High): gateway hooks (`HOOK.yaml` + `handler.py` in `~/.hermes/hooks/`, gateway only), Python plugin hooks (`ctx.register_hook()`), shell hooks (`hooks:` block in `~/.hermes/config.yaml`), outbound webhooks (`hooks.outbound:`). https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/hooks.md

**Plugin hook catalog with payloads (High, same URL).** The ones that matter for capture:
- `pre_api_request`: `session_id, model, provider, base_url, api_mode, api_call_count, retry_count, request_messages (unsanitized), request (sanitized), user_message, conversation_history, max_tokens, started_at, turn_id, api_request_id, task_id`. Fires per provider attempt. This is the closest thing to "exactly what was sent".
- `post_api_request`: `model, response_model, provider, base_url, usage, finish_reason, assistant_message, response (sanitized), api_duration, ...`.
- `api_request_error`, `transform_api_error_classification`.
- `pre_tool_call` (`tool_name, args, session_id, tool_call_id, turn_id, ...`; can block/modify), `post_tool_call` (adds `result` JSON string, `duration_ms`, `status`), `transform_tool_result`, `transform_terminal_output`.
- `pre_llm_call` (`session_id, user_message, conversation_history (full), model, platform, parent_session_id, sender_id, is_first_turn`; return injects context into the user message), `post_llm_call`, `transform_llm_output`.
- `on_session_start/end/finalize/reset`, `subagent_start` (`parent_session_id, child_session_id, child_role, child_goal, ...`), `subagent_stop`, `on_stream_start/delta/end` (`kind: text|reasoning`; reasoning deltas need `plugins.stream_reasoning_deltas: true`), `on_interim_message`, `pre_gateway_dispatch` (`event, gateway, session_store`; can skip/rewrite), `gateway_platform_event`, `pre_command` (slash commands), `pre_approval_request`, `post_approval_response`, `pre_verify`, `on_skill_lifecycle`, kanban hooks.
- Docs state: "Privacy: Full prompt/messages/tool results exposed to all hooks".
- `VALID_HOOKS` set in source begins `pre_tool_call, post_tool_call, transform_terminal_output, transform_tool_result, transform_llm_output, ...` — https://raw.githubusercontent.com/NousResearch/hermes-agent/main/hermes_cli/plugins.py

**Gateway hooks** (`agent:start`/`agent:end`) truncate `message`/`response` to 500 chars; not suitable for full capture. High.

**Shell hooks wire protocol (High).** stdin JSON `{hook_event_name, tool_name, tool_input, session_id, cwd, extra}`; stdout JSON to block/modify/inject. First-use consent stored in `~/.hermes/shell-hooks-allowlist.json`; bypass via `--accept-hooks`, `HERMES_ACCEPT_HOOKS=1`, or `hooks_auto_accept: true`. Note: "Script edits silently trusted (keyed on command string, not hash)". Shell hooks receive a *reduced* payload compared to Python hooks (no `request_messages`).

**Outbound webhooks (High).** HMAC-SHA256 signed POST with `delivery_id`/`timestamp`; payload mirrors the shell-hook envelope (`extra` carries event fields).

**Plugin locations/config (High).** Bundled `<repo>/plugins/`, user `~/.hermes/plugins/`, project `.hermes/plugins/` (needs `HERMES_ENABLE_PROJECT_PLUGINS=true`), pip entry points. Config under `plugins:` in `~/.hermes/config.yaml` (`enabled`, `disabled`, `hook_callback_timeout` default 30s, max 600, 0 disables). https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/plugins.md

**Mid-run editing / reload (High for docs, Medium for code paths).**
- Hooks doc: "Config reload: Changes take effect on next CLI session / gateway restart."
- Plugins load at start; docs page has no reload command; issue #15626 (open, P3) confirms "No reload capability exists today... requires a full gateway restart" for plugins. https://github.com/NousResearch/hermes-agent/issues/15626
- `/reload` refreshes `.env` only; `/reload-mcp` re-scans MCP servers; `/plugins` is display-only. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/reference/slash-commands.md
- Source note: `agent/shell_hooks.py` has `re_register_config_hooks()` "after plugin force-reload", and a `PluginManager.discover_and_load(force=True)` exists in-process (issue #15626), so an in-process reload primitive exists but no user-facing trigger is documented. A participant *can* edit `config.yaml` at any time; hook changes bite on the next session/gateway restart. Because `hermes -c`/`--resume` starts a new process against the same session id, a participant could disable hooks between restarts. Hot-reloaded config keys are limited to `model.context_length`, `compression.*`, `auxiliary.*`, `agent.disabled_toolsets`, `agent.agent_cache`. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/configuration.md

## 3. Telemetry / OTel / Langfuse / trajectory logging

- **No phone-home by default (High).** FAQ: "Hermes Agent does not collect telemetry, usage data, or analytics." https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/reference/faq.md
- **Built-in OTLP exporter is content-free (High).** `agent/monitoring/otlp_exporter.py` exports spans only for `gateway_health`, `gateway_diagnostic`, `cron_execution`; requires `monitoring.export.otlp.enabled: true` + `endpoint`; strings redacted and truncated to 500 chars; "content-free by construction". Useless for transcript capture. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/agent/monitoring/otlp_exporter.py , .../events.py , .../emitter.py
- **Langfuse bundled plugin (High).** `plugins/observability/langfuse/` — opt-in via `hermes plugins enable observability/langfuse`; env `HERMES_LANGFUSE_PUBLIC_KEY/SECRET_KEY/BASE_URL`; one trace per turn, one generation per API call, one observation per tool call; `HERMES_LANGFUSE_CAPTURE` = metadata | sanitized (default) | full; tool args/results and reasoning included in content modes; system prompt included where provider passes it separately. Subagent/gateway linkage not documented in README. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/plugins/observability/langfuse/README.md ; https://langfuse.com/integrations/other/hermes
- **Trajectory logging for training (High).** ShareGPT JSONL (`from: system|human|gpt|tool`), written to CWD as `trajectory_samples.jsonl` / `failed_trajectories.jsonl`; **disabled by default**; enabled by `--save_trajectories` (run_agent.py) or `AIAgent(save_trajectories=True)`; written once per completed trajectory, not per message; includes system prompt (regenerated at save time), reasoning normalized to `<think>`, tool results; batch_runner.py always writes. Not append-per-message, so weak as a live record. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/developer-guide/trajectory-format.md ; https://raw.githubusercontent.com/NousResearch/hermes-agent/main/agent/trajectory.py
- **HF trace upload (High).** `agent/trace_upload.py` / `hermes sessions export --format trace --upload`: Claude Code JSONL to a private `{user}/hermes-traces` dataset; skips `role == "system"`; redacted by default; opt-in only. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/agent/trace_upload.py

## 4. Headless / server modes

(High unless noted) https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/reference/cli-commands.md
- `hermes chat -q "..."` seeds an interactive session; `hermes chat --oneshot -q "..."` answers and exits; `-Q/--quiet` programmatic mode; `hermes -z "prompt"` "pure one-shot" (response only, `--usage-file <path>` writes JSON usage). No documented `--json`/stream-JSON event output for chat.
- `hermes gateway run|start|stop|status` — 30+ platforms. Cron runs "in a fresh agent session with no chat platform attached" and are recorded in `~/.hermes/cron/executions.db` + `~/.hermes/cron/output/{job_id}/{timestamp}.md`. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/cron.md
- **API server**: OpenAI-compatible `POST /v1/chat/completions`, `/v1/responses`, `/v1/runs` (event subscription), SSE streaming with custom `hermes.tool.progress`; enabled by `API_SERVER_ENABLED=true`, port 8642, bearer `API_SERVER_KEY`; creates state.db sessions. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/api-server.md
- **`hermes serve`**: headless `tui_gateway` JSON-RPC over WebSocket (port 9119) used by the desktop app and remote clients. Methods incl. `session.create/resume/list/active_list/branch/compress/interrupt`, `prompt.submit`, `approval.respond`, `llm.oneshot`, `config.set`, `reload.mcp`, `plugins.manage`, `shell.exec`; notifications incl. `message.interim`, `session.reclaimed`, stream deltas. Server keeps a `viewers` dict per session, so multiple clients can observe one live session (Medium — inferred from source summary, no doc). https://raw.githubusercontent.com/NousResearch/hermes-agent/main/tui_gateway/server.py ; https://hermes-agent.nousresearch.com/docs/developer-guide/programmatic-integration
- `hermes acp` — Agent Client Protocol stdio server for editors (Zed etc.); sessions persisted with `source="acp"`. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/acp_adapter/session.py
- `hermes dashboard` — browser UI on 127.0.0.1:9119; Chat tab spawns `hermes --tui` behind a PTY; REST `GET /api/sessions`, `WebSocket /api/pty`. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/web-dashboard.md
- Export: `hermes sessions export` (see §1). `hermes sessions import` imports Claude Code/Codex transcripts (i.e., writes arbitrary sessions into state.db).

## 5. Model identity

- Config default: `model.provider` / model set via `hermes model` wizard; `-m/--model`, `--provider` per-run; `HERMES_INFERENCE_MODEL` env. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/reference/cli-commands.md (High)
- Per session: `sessions.model`, `model_config`, `billing_provider`, `billing_base_url`, `billing_mode`. Per model within a session: `session_model_usage` rows (model × provider × base_url × task, with first_seen/last_seen and call counts). **Not per message.** (High, schema)
- `/model` mid-run: session-only unless `--global` or `model.persist_switch_by_default: true`; "resets the prompt cache". Not recorded as a message; only visible as a new `session_model_usage` row and via hooks. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/reference/slash-commands.md (High)
- Hooks: `pre_api_request` / `post_api_request` carry `model`, `provider`, `base_url`, `api_mode`, and `response_model` (server-reported) per API call — the only per-call model identity source. (High)
- Subagents: inherit parent model unless `delegation.model/provider/base_url` set; recorded on the child's own session row; `subagent_start` hook gives child ids; `SubagentHandle` exposes child `model`/`provider`. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/delegation.md ; https://raw.githubusercontent.com/NousResearch/hermes-agent/main/agent/subagent_lifecycle.py (High)

## 6. Human vs harness-authored messages

- Role alone is insufficient: several harness paths write `role=user` rows (High):
  - Cron mirror into a chat session: "written as a labelled user turn (`[Cron delivery: <task name>]`)" with `source_label="cron"`. Cron job prompts themselves live in separate cron sessions and "are NOT mirrored into gateway session history". https://raw.githubusercontent.com/NousResearch/hermes-agent/main/cron/scheduler.py ; https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/developer-guide/gateway-internals.md
  - `pre_llm_call` plugin context is "appended to user message" but is ephemeral; the wire bytes go to `api_content` when they differ from `content`. https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/features/hooks.md ; session-storage doc (Medium)
  - Compaction summaries: `role` row with `_compressed_summary=1` and `[CONTEXT COMPACTION]` prefix. (High)
  - `/steer`, `/queue`, `/bg` inject user-role content; `/bg` runs a separate background session. (High, slash-commands doc)
  - Gateway group chats: triggered message tagged `[nickname|user_id]` plus a per-turn safety prompt; optional `gateway.message_timestamps.enabled` prepends `[Tue 2026-04-28 13:40:53 CEST]` in-context only ("Persisted transcripts always stay clean — the timestamp is stored as message metadata"). https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/messaging/telegram.md ; https://hermes-agent.nousresearch.com/docs/user-guide/messaging/
  - Memory: injected only into the system prompt at session start (never as a user turn). (High)
- Session-level provenance: `sessions.source` (cli, telegram, discord, acp, ...; full enum not documented), `user_id`, `chat_id`, `platform_message_id` per message (gateway), `origin_json`, `display_kind`/`display_metadata` (undocumented semantics). (Medium)
- The unambiguous signal for "a human typed this": a `role=user` row with `active=1`, no `_compressed_summary`, no `[Cron delivery:` prefix, in a session whose `source` is the participant's declared surface, **and** a matching `pre_gateway_dispatch`/`pre_llm_call` hook event whose `user_message` equals the row's `content`, and (for gateway) a `platform_message_id`. There is no built-in `is_human` flag. (Medium — synthesis)

## 7. Third-party recorders/exporters

(Low–Medium; existence verified, capabilities from their own pages)
- jazzyalex/agent-sessions — macOS browser for `state.db` (read-only; falls back to legacy JSON). https://jazzyalex.github.io/agent-sessions/guides/hermes-agent-state-db-history.html
- ccusage Hermes data source — reads `$HERMES_HOME/state.db` token/cost rows. https://ccusage.com/guide/hermes/
- EKKOLearnAI/hermes-studio (dashboard over local SQLite sessions), sanchomuzax/hermes-webui ("full inspection of conversation histories"), liaohch3/claude-tap (API-traffic interceptor — requires a proxy, so out of scope here), junhoyeo/tokscale. https://hermesatlas.com/lists/workspaces-and-guis ; https://hermesatlas.com/lists/developer-tools
- Langfuse and SigNoz integration guides (see §3). https://signoz.io/docs/hermes-monitoring/
- I found no third-party tool that produces a tamper-evident or append-only capture.

## 8. Launch modes and observability by a local tracker

| Mode | Exists | Writes to `~/.hermes/state.db` | Python hooks fire | Tracker can observe |
|---|---|---|---|---|
| Classic CLI (`hermes`, `hermes chat -q/--oneshot/-Q`) | High | High | High | Yes |
| TUI (`hermes --tui`, Ink subprocess ↔ Python tui_gateway over stdio) | High | High ("both write to the same state.db") | High | Yes |
| `hermes -z` one-shot | High | **Not verified** | Unknown | Probably (state.db) |
| Gateway (Telegram/Discord/Slack/WhatsApp/Signal/... 30+) | High | High | High (hooks "fire in both CLI and gateway") | Yes |
| API server (OpenAI-compatible, port 8642) | High | High | High (same AIAgent) | Yes |
| `hermes serve` + **Electron desktop app** (macOS/Windows/Linux) | High | High ("same conversation history as every other Hermes surface") | High | Yes, if `hermes serve` runs on the tracked machine |
| Desktop app → **remote** `hermes serve` | High | On the remote host | On the remote host | Only if tracker runs on the remote host |
| Web dashboard (`hermes dashboard`, 9119) | High | High | High | Yes |
| ACP (`hermes acp`, Zed/VS Code) | High | High (`source="acp"`) | Medium | Yes |
| Docker (`nousresearch/hermes-agent`, `-v ~/.hermes:/opt/data`, `HERMES_HOME=/opt/data`) | High | Host-mounted | Inside container | Yes via mounted volume; hooks must be installed inside the container's HERMES_HOME |
| Python library (`AIAgent`) | High | Not documented | Medium | Uncertain |
| Remote terminal backends (SSH/Modal/Daytona/Vercel) | High | Local state.db (only tool execution is remote) | High | Yes |

Sources: https://raw.githubusercontent.com/NousResearch/hermes-agent/main/website/docs/user-guide/desktop.md ; .../user-guide/tui.md ; .../user-guide/docker.md ; .../user-guide/features/web-dashboard.md ; .../guides/python-library.md ; .../reference/cli-commands.md

## 9. Tamper surface

- `state.db` is plain SQLite; no row checksums, no chained hashes; only `system_prompt_hash` (SHA-256 for dedup). Sessions/messages can be hard-deleted (`hermes sessions delete/prune`), soft-hidden (`archive`, `hidden`), rewound (`active=0`), imported wholesale (`hermes sessions import`). `id` is AUTOINCREMENT, which leaves gaps on deletion — weak evidence, not proof. (High) https://raw.githubusercontent.com/NousResearch/hermes-agent/main/hermes_state_common.py ; sessions.md
- Shell hooks are trusted by command string, not content hash ("Script edits silently trusted"). Hook config lives in the participant-writable `config.yaml`. (High)
- Delegation live logs and cron output are plain text/markdown. (High)
- Outbound webhooks are HMAC-signed with `delivery_id`/`timestamp` — the only integrity primitive in the system, and it's per event, not per transcript. (High)
- `hermes plugins` has `scan_on_install` and capability consent, but a participant controls `~/.hermes/plugins/` and `plugins.disabled`. (High)

---

## Recommended capture design

1. **Primary: a Python plugin in `~/.hermes/plugins/<tracker>/`** registering `pre_api_request`, `post_api_request`, `api_request_error`, `pre_tool_call`, `post_tool_call`, `pre_llm_call`, `on_session_start/end/reset/finalize`, `subagent_start/stop`, `pre_gateway_dispatch`, `pre_command`, `post_approval_response`, `on_stream_delta` (with `plugins.stream_reasoning_deltas: true`). Persist `request_messages`/`request` verbatim from `pre_api_request` — that is the actual per-call context sent to the provider, including model/provider/base_url, and it covers subagents (same AIAgent core) and mid-run `/model` changes (`response_model` per call). Write to an append-only, per-event hash-chained JSONL outside `~/.hermes` (root/other-user-owned directory), ship to organizer storage continuously (private). Confidence the payloads exist as documented: High; confidence they fire on every surface incl. `-z` and library use: Medium.
2. **Secondary: periodic snapshot of `~/.hermes/state.db`** (WAL-safe `sqlite3 .backup` or `VACUUM INTO`) plus `~/.hermes/cache/delegation/live/`, `~/.hermes/cron/`, `~/.hermes/logs/`, `config.yaml`, `plugins/`, `hooks/`, `shell-hooks-allowlist.json`. Diffing snapshots detects deletes/rewinds/imports and config/hook edits. High.
3. **Tertiary: outbound webhook** (`hooks.outbound`) to an organizer endpoint for `on_session_end`, `subagent_stop`, `pre_command` etc. as an independent, HMAC-signed heartbeat that would go silent if a participant disabled hooks. High.
4. **Human-turn rule**: a "human" message = `pre_gateway_dispatch`/`pre_llm_call` `user_message` matching a `role=user` row with `active=1`, `_compressed_summary=0`, no `[Cron delivery:` prefix; anything else in `content`/`api_content` that is not the initial prompt or `keep going` is a violation. Also flag any `pre_command` other than an allowlist, any `/bg`, `/steer`, `/queue`, `/undo`, `/model`, and any `session.branch`/rewind (`rewind_count`, `active=0` rows). Medium.
5. **Lock the config**: require `plugins.enabled` to include the tracker, `hooks_auto_accept: true`, `hooks.outbound` set; record `config.yaml` + plugin dir hash at start and on each snapshot; treat a gateway/CLI restart without the tracker plugin loading (`on_session_start` missing while state.db grows) as a break. Medium.
6. Do **not** rely on: built-in OTLP (content-free), `--save_trajectories` (end-of-run only, CWD file), gateway `HOOK.yaml` hooks (500-char truncation), `agent.log` (redacted, rotates at 5 MiB × 3).

## Unobservable modes

- Desktop app or any client pointed at a **remote `hermes serve`** on an untracked host (High).
- Participant running a second, untracked `HERMES_HOME` / `--profile` / Docker container with a different volume (High) — detect by `sessions.profile_name`, `install_id`, and by comparing provider-side usage where available.
- `--ignore-user-config` / `--safe-mode` starts, which skip `~/.hermes/config.yaml` and "disable all customizations" — likely skipping plugins/hooks (Medium; not verified whether plugins load under `--safe-mode`).
- `AIAgent` used directly as a Python library with `quiet_mode`/`skip_memory`: persistence to state.db not documented (Medium).
- Anything a participant does to the model *outside Hermes* with the same subscription (out of scope for this tracker).

## Unknowns / could not verify

- Whether `hermes -z` and `--oneshot` sessions persist to `state.db` (Low).
- Whether `sessions.system_prompt` is rewritten when compaction "rebuilds" the system prompt, or stays the session-start snapshot (Low).
- Whether `pre_api_request.request`/`request_messages` include the system message for Anthropic-style providers (system passed as a separate parameter) (Medium that `request` does; not verified).
- Exact enumerated values of `sessions.source` (delegate children? cron? api_server?) and semantics of `display_kind`, `display_metadata`, `effect_disposition`, `observed`, `origin_json` (Low).
- Whether Python plugin hooks load under `--safe-mode` / `--ignore-user-config` (Low).
- Whether any in-process plugin reload path is reachable by users today; issue #15626 says none, `discover_and_load(force=True)` exists internally (Medium).
- Langfuse plugin coverage of subagent and gateway sessions (README silent; issue #1501 requested it) (Low).
- Whether `hermes_cli/plugins.py` `VALID_HOOKS` contains more names than the docs' 26 + kanban set (the file summary claimed ~75; I could only confirm the documented ones) (Low).
