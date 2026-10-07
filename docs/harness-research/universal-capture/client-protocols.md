# "Tracker as client": driving harnesses through their client protocols

Research date: 2026-09-10. Local check: `claude` 2.1.259 and `codex` 0.143.0 are installed on this Mac (help output inspected); `opencode`, `hermes`, `openclaw`, `agent`/`cursor-agent`, `gemini` are not installed, so those rows rely on docs only.

Labels: **V** = Verified against a primary or near-primary source (URL given). **I** = Inferred from verified facts. **U** = Unknown / not documented.

---

## 0. The protocols on offer

### ACP (Agent Client Protocol), agentclientprotocol.com
- JSON-RPC 2.0 over stdio; `protocolVersion: 1` (integer MAJOR, bumped only on breaking changes; features are added via capability negotiation). **V** https://agentclientprotocol.com/protocol/v1/initialization
- Session methods: `session/new`, `session/load` (optional, agent MUST replay whole history as `session/update`), `session/resume` (RFD stabilised 2026-04-22: reconnect without replay; capability `sessionCapabilities.resume`), `session/prompt`, `session/cancel`, `session/request_permission` (agent→client). **V** https://agentclientprotocol.com/protocol/session-setup , https://agentclientprotocol.com/rfds/session-resume
- `session/update` variants: `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `current_mode_update`, `usage_update` (used/size/cost), `config_option_update`, `session_info_update`. **V** https://agentclientprotocol.com/protocol/schema , https://agentclientprotocol.com/protocol/prompt-turn
- ToolCall: `toolCallId`, `title`, `kind`, `status`, `content` (text/diff/terminal blocks), `locations`, `rawInput`, `rawOutput`. **`rawInput`/`rawOutput` are optional**; the spec does not require the full raw output to be carried. **V** https://agentclientprotocol.com/protocol/tool-calls
- No first-class field for "model id per turn" or "system prompt"; model selection is exposed only via agent-defined `SessionConfigOption`s. **V** (schema) / **I** (for the gap)
- Governance: repo moved to the `agentclientprotocol` GitHub org (Zed-originated, JetBrains co-develops since Oct 2025; Python/Rust/TS SDKs; ACP Registry since Jan 2026). **V** https://github.com/agentclientprotocol/agent-client-protocol , https://www.jetbrains.com/acp/

### MCP sampling / elicitation (option c)
- Sampling (`sampling/createMessage`) would let an MCP server ask the host's model for completions; it is the wrong direction for a tracker and Claude Code and Codex both have it as open feature requests, not shipped. **V** https://github.com/anthropics/claude-code/issues/1785 , https://github.com/openai/codex/issues/4929
- Elicitation is supported by Claude Code (`Elicitation` hook) and Codex (`mcpServer/elicitation/request`) but only lets a server ask the user a form question; it does not expose the transcript. **V** https://code.claude.com/docs/en/headless , https://learn.chatgpt.com/docs/app-server
- Verdict: not relevant as a tracking surface. **I**

---

## 1. Per-harness findings

### 1.1 Claude Code CLI

| Aspect | Finding |
|---|---|
| Protocols | (a) `claude -p --output-format stream-json --input-format stream-json` (+ Python/TS Agent SDK wrapping the same CLI). NDJSON, documented per-field with version gates (e.g. `capabilities` array in `system/init` since v2.1.205 for feature detection). **V** https://code.claude.com/docs/en/headless (b) ACP via `@agentclientprotocol/claude-agent-acp`, which wraps the Agent SDK, not the CLI. **V** https://github.com/agentclientprotocol/claude-agent-acp |
| Maintainer | Native: Anthropic. ACP adapter: originally Zed, now the `agentclientprotocol` org (community/Zed, not Anthropic). **V** https://zed.dev/blog/acp-progress-report |
| Completeness (stream-json) | `system/init` (model, tools, MCP servers, plugins, permission mode); `assistant` messages carry the API message incl. `model`, `usage`, thinking blocks (when thinking on), tool_use blocks with full input; `user` messages carry `tool_result` content as sent to the model; `stream_event` deltas with `--include-partial-messages`; `system/compact_boundary` with `compact_metadata` (trigger, pre_tokens); `system/api_retry`; `system/permission_denied`; `result` with `usage`, `modelUsage`, `total_cost_usd`, `permission_denials`; subagent messages tagged `parent_tool_use_id` at every nesting depth (tool_use/tool_result by default; text+thinking with `--forward-subagent-text`, v2.1.211+); hook lifecycle events with `--include-hook-events`; stdin user messages echoed with `--replay-user-messages`. **V** https://code.claude.com/docs/en/headless , https://code.claude.com/docs/en/agent-sdk/typescript , local `claude --help` |
| Completeness gaps | System prompt text is never emitted (only "snapshot" behaviour is controllable with `--system-prompt-snapshot`). Compaction summary text: the boundary event is emitted, the summary itself arrives as the next context but is not documented as a distinct payload. **I/U** Tool results are what the model received (Claude Code's own truncation applies equally to the on-disk transcript). **I** |
| Behaviour vs TUI | Without `--bare`, `-p` loads the same system prompt, tools, hooks, skills, plugins, subagents, CLAUDE.md, MCP servers and auto-compaction as interactive. Documented differences: no workspace-trust dialog; permission prompts must be answered by the SDK host/`--permission-prompt-tool` or are denied; `AskUserQuestion` removed under `--permission-prompts none`; terminal-only slash commands (`/login`, `/plugin`, `/resume` picker) unavailable; background subagents/workflows waited on up to a 10-minute idle ceiling (`CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`), background Bash killed 5 s after the final result; Anthropic says `--bare` "will become the default for -p in a future release" (a future behaviour-change risk). **V** https://code.claude.com/docs/en/headless |
| Other input paths | Remote Control (claude.ai/code, mobile app) attaches only to interactive sessions started with `--remote-control`, `/remote-control` or `claude remote-control` server mode; messages from phone/browser are synced into the local transcript and queued mid-turn. `-p` sessions cannot be reached by Remote Control, `claude attach` (`--bg`), Dispatch or `/teleport`. `--cloud <id>` with `-p` sends to a *cloud* session, not a local one. Resuming the same session in a second terminal is possible (`--resume`) but the stream-json client would not see that turn. **V** https://code.claude.com/docs/en/remote-control , https://code.claude.com/docs/en/headless ; sole-stdin claim **I** |
| Resume / long run | `--resume <id>` (or transcript path) restores full context, permission mode, model; SIGTERM leaves the turn unfinished and resume continues it; `--session-id` lets the tracker pre-choose the ID. No documented session-duration limit. **V** https://code.claude.com/docs/en/headless |
| Subscription auth | Docs: `-p` works with a Claude account login. Anthropic planned (May 13) to move `claude -p`/SDK/third-party usage to a separate "Agent SDK credit" pool ($20/$100/$200 per month) on June 15 2026 but **paused that change**: "For now, nothing has changed: Claude Agent SDK, `claude -p`, and third-party app usage still draw from your subscription's usage limits." **V** https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan . ToS: OAuth is for "ordinary use of Claude Code and other native Anthropic applications"; third parties may not route requests through Pro/Max credentials or intermediate tokens; an unmodified `claude` binary signed in by its own user is explicitly allowed even when a platform hosts it. **V** https://code.claude.com/docs/en/legal-and-compliance . The ACP adapter (Agent SDK inside a third-party process) sits in the grey zone; issue #337 was closed without a maintainer answer. **V** https://github.com/agentclientprotocol/claude-agent-acp/issues/337 |
| ACP adapter fidelity | Supports tool calls with permission requests, nested subagent transcripts, terminals, custom slash commands, client MCP servers; **hooks not supported, agent teams not supported**; Zed lists editing past messages/resuming from history/checkpointing as unavailable. Subagent sessions need a draft capability (`clientCapabilities.subagents` / `_meta.jetbrains.air.capabilities.nativeSubagentSessions`). **V** https://zed.dev/acp/agent/claude-agent (via search summary), https://github.com/agentclientprotocol/claude-agent-acp |

### 1.2 Codex CLI

| Aspect | Finding |
|---|---|
| Protocols | (a) `codex app-server` JSON-RPC 2.0 over stdio (stable), unix socket, or WebSocket (experimental). `generate-ts` / `generate-json-schema` emit version-matched bindings. Experimental methods gated behind `capabilities.experimentalApi`. (b) `codex exec --json` JSONL (thin). (c) ACP via `@agentclientprotocol/codex-acp`, which itself wraps the app-server. **V** https://learn.chatgpt.com/docs/app-server , local `codex app-server --help`, `codex exec --help`, https://github.com/agentclientprotocol/codex-acp |
| Maintainer | app-server: OpenAI (powers desktop app, VS Code, web, and the TUI via an in-process transport). codex-acp: `agentclientprotocol` org, originally Zed. **V** https://codex.danielvaughan.com/2026/04/15/codex-app-server-complete-guide/ (TUI in-process claim), https://zed.dev/blog/acp-progress-report |
| Completeness (app-server) | `item/started` / `item/completed` for `userMessage`, `agentMessage`, `reasoning` (summaries and raw blocks when the model supports it), `commandExecution` (status, exit code, output), `fileChange` (diff), `mcpToolCall` (arguments, result), `webSearch`, `plan`, `contextCompaction`; deltas for agent text, reasoning summary, command output; `turn/tokenUsage/updated`; `turn/plan/updated`; `turn/diff/updated`; approval requests as server→client JSON-RPC requests; `thread/list` with `sourceKinds` incl. `subAgent*`. **V** https://learn.chatgpt.com/docs/app-server |
| Completeness gaps | Command output has configurable per-stream byte caps with a `capReached` marker; `exec --json` caps `aggregated_output` at 64 KiB and does not emit the model id or echo the user message. Model id per turn: present on `thread/start`/`turn/start` params, not documented as a per-item field. System prompt / `developer_instructions` not emitted. Subagent turns appear as separate threads (`sourceKinds: subAgent*`); a client must subscribe to them. **V** https://takopi.dev/reference/runners/codex/exec-json-cheatsheet/ ; per-turn model **U** |
| Behaviour vs TUI | The TUI is an app-server client (in-process transport), so the loop, hooks (`hooks.json`), skills, subagents (explicit `/agent` only) and MCP config are filesystem-resident and apply to every client. A custom client must supply its own `approvalPolicy`/`sandboxPolicy` (defaults come from config.toml). **V** https://gist.github.com/oneryalcin/ee2c27e2d8aa040da8fbe7eebcc2ecea , https://learn.chatgpt.com/docs/app-server ; parity **I** |
| Other input paths | Multiple clients can `thread/resume` (subscribe to) the same thread on one app-server and `turn/steer` it; the daemon (`codex app-server daemon`, `codex remote-control`) exposes threads to the Codex desktop app and the ChatGPT mobile app via OpenAI's relay once "Allow other devices to connect" is enabled (off by default); `codex --remote ws://…` attaches a TUI to a remote app-server. If the tracker spawns its **own** app-server over stdio (not the daemon), only stdio clients exist. **V** https://learn.chatgpt.com/docs/app-server , https://codex.danielvaughan.com/2026/05/21/codex-remote-connections-mobile-pairing-ssh-hosts-enterprise-access-tokens/ , local `codex remote-control --help` |
| Resume / long run | Threads persist as rollout JSONL in `~/.codex/sessions/`; `thread/resume` by id restores history; `thread/fork`; inactive threads unload after 30 min idle (`thread/closed`) and reload on resume. **V** https://learn.chatgpt.com/docs/app-server |
| Subscription auth | app-server has `account/read` / login flows for ChatGPT auth; the desktop app is itself an app-server client on ChatGPT plans, and codex-acp documents "ChatGPT browser-based login" plus `CODEX_API_KEY`. No published ToS restriction on headless use with a ChatGPT plan was found. **V** https://github.com/agentclientprotocol/codex-acp ; ToS **U** |

### 1.3 OpenCode

| Aspect | Finding |
|---|---|
| Protocols | HTTP + SSE server (`opencode serve`, OpenAPI 3.1 spec at runtime, generated JS SDK); `/event` SSE bus; `opencode run --format json`; `opencode acp` native ACP server. **V** https://opencode.ai/docs/server/ , https://opencode.ai/docs/cli/ , https://zed.dev/acp/agent/opencode |
| Maintainer | anomalyco (vendor) for server, SDK and ACP. **V** |
| Completeness | Events: `message.updated` (AssistantMessage has `modelID`, `providerID`, tokens incl. reasoning and cache, `cost`, system prompt reference, error), `message.part.updated` (TextPart, FilePart, ToolPart with state pending→running→completed/error carrying `input`, `output`, `title`, `metadata`, time; StepStart/StepFinish with token metrics; SnapshotPart; PatchPart), `session.updated/idle/error`, `permission.updated`, `file.edited`. **V** https://deepwiki.com/anomalyco/opencode-sdk-js/4.4-event-resource , https://deepwiki.com/anomalyco/opencode-sdk-js/4.1.1-message-and-part-types |
| Completeness gaps | Reasoning parts and compaction/subtask parts not in the SDK page I could verify (**U**); truncation of tool output in parts undocumented (**U**). Because the TUI is a client of the same server and reads the same store, the SSE stream equals what the TUI sees. **I** |
| Behaviour vs TUI | None expected: "the TUI is the client that talks to the server"; `opencode attach <url>` connects a TUI to a running server. **V** https://opencode.ai/docs/server/ , https://opencode.ai/docs/cli/ |
| Other input paths | Anyone with the server URL/basic-auth (`OPENCODE_SERVER_PASSWORD`) can POST `/session/:id/message`; `opencode attach`, `opencode web`, IDE plugins and `/tui` endpoints are all peers. The tracker cannot be the sole client unless it owns the port and credentials and the session key. **V** |
| Resume / long run | Sessions persisted server-side; `--session`, `--continue`, `--fork` on attach/run. **V** https://opencode.ai/docs/cli/ |
| Subscription auth | Claude Pro/Max OAuth blocked server-side since Jan 9 2026 and by ToS since Feb 20 2026; the May 13 "reinstatement" was via Agent SDK credits, then paused June 15 (see 1.1). ChatGPT-plan OAuth: OpenAI added ChatGPT OAuth for third-party agents in Mar 2026 but OpenCode users report the option vanishing/breaking. **V** https://code.claude.com/docs/en/legal-and-compliance , https://venturebeat.com/technology/anthropic-reinstates-openclaw-and-third-party-agent-usage-on-claude-subscriptions-with-a-catch , https://github.com/anomalyco/opencode/issues/16316 |

### 1.4 Hermes (Nous Research)

| Aspect | Finding |
|---|---|
| Protocols | `hermes acp` (ACP stdio server: new/load/fork/list sessions, prompt, tool events, reasoning via step callbacks, permission requests, cancel); TUI-gateway JSON-RPC (`tui_gateway/server.py`, stdio or WebSocket; 40+ methods incl. `prompt.submit`, `session.steer`, `session.history`, `tool.start/progress/complete`, `approval.request`, `clarify.request`; events `message.delta`, `message.complete`); OpenAI-compatible API server (`/v1/chat/completions`, `/v1/responses` with `function_call`/`function_call_output` items, `/v1/runs`); `hermes -z` one-shot; direct Python import. **V** https://hermes-agent.nousresearch.com/docs/developer-guide/programmatic-integration , https://hermes-agent.nousresearch.com/docs/developer-guide/acp-internals , https://github.com/nousresearch/hermes-agent/blob/main/website/docs/reference/cli-commands.md |
| Maintainer | Nous Research (vendor) for all of the above. **V** |
| Completeness | "All protocols drive the same underlying AIAgent core." ACP forwards tool progress and reasoning; TUI gateway exposes the full feature set (slash commands, approvals, clarify, multi-agent, branching). `--usage-file` writes token/cost JSON per run. Model id per turn and full untruncated tool output are not documented for ACP (**U**); `chat/completions` stream only carries `hermes.tool.progress` (tool-start UX), while the Responses API replays tool calls/outputs. **V/U** |
| Behaviour vs TUI | Same core loop; ACP ignores non-text prompt blocks; no `--mode rpc`. **V** |
| Other input paths | The TUI "spawns its own in-process gateway" and "there is no general 'point any TUI at any standalone gateway port' mode", but TUI and classic CLI share `~/.hermes/state.db`, so a second CLI can resume the same session; the messaging gateway (Telegram/Discord/Slack/WhatsApp/…) is a separate service with its own platform-keyed sessions in the same DB (whether it can be pointed at the ACP session is **U**). **V** https://hermes-agent.nousresearch.com/docs/user-guide/tui |
| Resume / long run | ACP sessions persist in `state.db`, survive restarts, support `session/load`/fork. **V** |
| Subscription auth | ACP "reuses Hermes' runtime provider … advertises the currently configured Hermes provider/credentials"; Nous Portal subscription applies. Using a Claude Max login inside Hermes is subject to Anthropic's third-party ban. **V/I** |

### 1.5 OpenClaw

| Aspect | Finding |
|---|---|
| Protocols | Gateway WebSocket JSON protocol v4 (~200 RPC methods; `chat.send` with queue modes steer/followup/collect/interrupt, `chat.history`, `agent`, `agent.wait`, `sessions.list`; events `chat`, `session.message`, `session.tool`, `session.observer`, `sessions.changed`); `openclaw acp` bridge translating ACP↔Gateway. **V** https://docs.openclaw.ai/gateway/protocol/rpc-methods , https://docs.openclaw.ai/cli/acp |
| Maintainer | OpenClaw project (vendor) for both. **V** |
| Completeness | ACP bridge emits text, `agent_thought_chunk`, `tool_call`/`tool_call_update` with raw I/O, `session_info_update`, `usage_update` ("approximate"). Third-party audit lists: session history not replayed on `loadSession` for non-bridge sessions, MCP configs dropped, lossy stop reasons, mode changes silently succeed, no permission-request flow, tool calls lack diffs/locations/plan/thoughts in some paths. `chat.history` "strips directive tags and tool-call payloads" (display-normalised), so history is not a faithful transcript. **V** https://docs.openclaw.ai/cli/acp , https://shashikantjagtap.net/openclaw-acp-what-coding-agent-users-need-to-know-about-protocol-gaps/ |
| Behaviour vs TUI | CLI, macOS app, web UI, automations are all Gateway WS clients, so the loop is the same; the ACP bridge disables per-session MCP servers and client fs/terminal methods. **V** https://docs.openclaw.ai/concepts/architecture |
| Other input paths | Everything: any paired operator client, the web UI, macOS app, mobile nodes, and every messaging channel (WhatsApp, Telegram, Slack, Discord, Signal, iMessage, WebChat) route into Gateway sessions; concurrent ACP clients on one session key are "best-effort rather than strictly isolated". The tracker cannot be a chokepoint without disabling channels and pairing. **V** |
| Resume / long run | ACP `resumeSession` rebinds without replay; `loadSession` replays only bridge-created sessions; Gateway sessions persist. **V** |
| Subscription auth | Anthropic explicitly blocked OpenClaw on Claude subscriptions (Apr 4 2026), then re-allowed via Agent SDK credits (May 13) whose rollout was paused June 15; current state: subscription-limit-drawn, policy still in flux. ChatGPT OAuth for OpenClaw exists (Mar 2026). **V** https://venturebeat.com/technology/anthropic-reinstates-openclaw-and-third-party-agent-usage-on-claude-subscriptions-with-a-catch , https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan |

### 1.6 Cursor CLI (`agent`)

| Aspect | Finding |
|---|---|
| Protocols | `agent -p --output-format stream-json` (NDJSON: `system/init` with model, permissionMode, session_id; `user`; `assistant`; `tool_call` started/completed with typed tool payload and `result`; `result` with durations); `--stream-partial-output`. Official `agent acp` ACP server (session/new, session/load, modes agent/plan/ask, `session/request_permission`, MCP from `.cursor/mcp.json`). **V** https://cursor.com/docs/cli/reference/output-format , https://cursor.com/docs/cli/acp |
| Maintainer | Cursor (vendor) for both; several community ACP adapters predate it. **V** https://zed.dev/acp/agent/cursor |
| Completeness gaps | Docs: thinking "suppressed in print mode"; no token usage, no subagent events, no compaction events documented in stream-json; tool `result` is a typed summary (e.g. lines created, file size), not guaranteed raw output. **V/U** https://cursor.com/docs/cli/reference/output-format |
| Behaviour vs TUI | Print mode requires `--force` to apply edits (else changes are only proposed); ACP docs say ACP "is intended for building custom clients … for normal terminal workflows use the interactive CLI". Team-level MCP servers unavailable over ACP. **V** https://cursor.com/docs/cli/headless , https://cursor.com/docs/cli/acp |
| Other input paths | None documented for a `-p`/ACP process (no remote control equivalent found). Cursor's cloud "Background Agents" are a separate product. **U/I** |
| Resume / long run | `--resume` exists for the CLI; ACP `session/load` supported. Long-run limits **U**. **V** |
| Subscription auth | `agent login`, `CURSOR_API_KEY`, `CURSOR_AUTH_TOKEN`; plans state CLI/SDK usage draws from the plan's credit pools (some regional plans exclude the SDK). No ToS ban on headless use found. **V** https://cursor.com/docs/cli/acp ; billing **I** |

### 1.7 Gemini CLI (bonus)

| Aspect | Finding |
|---|---|
| Protocols | `--output-format stream-json` (events `init` with session id + model, `message`, `tool_use` with arguments, `tool_result` output, `error`, `result` with per-model token stats); `gemini --acp` native ACP (reference implementation; load/resume, approval-mode switching, model switching, proxied FS). **V** https://geminicli.com/docs/cli/headless/ , https://geminicli.com/docs/cli/acp-mode/ |
| Maintainer | Google (vendor) for both. **V** |
| Completeness gaps | Thinking not listed as an event; subagents not listed; compaction not listed. **U** |
| Behaviour vs TUI | Headless triggers on non-TTY or `-p`; auth must be cached or env-based; otherwise same core. Product-status caveat: one search snippet claims Google retired Gemini CLI for free users on 2026-06-18 in favour of "Antigravity CLI (agy)" — **unverified**, treat as a risk to check. |
| Other input paths | None documented for headless/ACP. **U** |
| Resume / long run | `-r latest` / `-r <id>` work in headless; session id now included in JSON output (issue #14435 fixed). **V** https://github.com/google-gemini/gemini-cli/issues/14435 |
| Subscription auth | Google login OAuth works when credentials are cached; Zed lists "Log in with Google (recommended)" for ACP. **V** https://zed.dev/acp/agent/gemini-cli |

---

## 2. Cross-harness summary table

| Harness | Best client protocol | Maintainer | Completeness gaps (vs own transcript) | Behaviour differences | Other human input paths | Resume | Sub auth OK |
|---|---|---|---|---|---|---|---|
| Claude Code | stream-json in/out (`-p`) **V** | Anthropic | System prompt text; compaction summary payload **I/U** | None without `--bare`; no trust dialog; AskUserQuestion may be removed; `--bare` may become default later **V** | None for `-p` (Remote Control/attach/Dispatch are interactive-only) **V/I** | `--resume`, `--session-id` **V** | Yes, draws from plan limits (credit-pool change paused) **V**; ACP adapter = grey zone |
| Codex | app-server JSON-RPC (stdio) **V** | OpenAI | Output byte caps; per-turn model id **U**; subagents are separate threads | None (TUI is an app-server client) **I** | Desktop/mobile via daemon+remote control (opt-in); other stdio clients none **V** | `thread/resume` **V** | ChatGPT login supported **V** |
| OpenCode | HTTP+SSE `/event` **V** | anomalyco | Reasoning/compaction parts **U** | None (TUI is a server client) **V** | Any HTTP client, `attach`, `web`, IDE **V** | server-side sessions **V** | Claude sub: blocked; ChatGPT: flaky **V** |
| Hermes | TUI-gateway JSON-RPC or ACP **V** | Nous | Model/usage per turn over ACP **U**; chat/completions stream thin | Same core loop **V** | Second CLI on same state.db; messaging gateway sessions separate **V/U** | state.db, `session/load` **V** | Nous Portal yes; Claude sub subject to ban **V/I** |
| OpenClaw | Gateway WS v4 (or ACP bridge) **V** | OpenClaw | ACP bridge drops plan/diffs, lossy stop reasons; history normalised **V** | Same loop; ACP bridge disables per-session MCP/fs/terminal **V** | Every channel + every paired client **V** | resume/load partial **V** | Claude sub: in flux; ChatGPT OAuth yes **V** |
| Cursor CLI | stream-json or official `agent acp` **V** | Cursor | Thinking suppressed; no usage; subagents **U** | Needs `--force`; ACP "for custom clients" **V** | None found **U** | `--resume`, `session/load` **V** | Plan credits **V/I** |
| Gemini CLI | stream-json or native `--acp` **V** | Google | Thinking/subagents/compaction **U** | Minimal **I** | None found **U** | `-r <id>` **V** | Google OAuth cached **V** |

---

## 3. Who else built on ACP and what they report

- Clients: Zed, JetBrains AI Assistant 2026.2 (no WSL; no listed fidelity caveats), Neovim (CodeCompanion, avante.nvim, agentic.nvim), Emacs agent-shell (context-usage indicator built on `usage_update`; author says implementation "hardly complete"), marimo (`use-acp` React hooks), Kiro, Goose, Toad, Eclipse prototype, Vercel AI SDK community provider, ACPX (scriptable CLI client for multi-agent orchestration). **V** https://zed.dev/blog/acp-progress-report , https://www.jetbrains.com/help/ai-assistant/acp.html , https://xenodium.com/introducing-agent-shell , https://casys.ai/blog/acpx-multi-agent-orchestration
- Reported gaps: ACPX: Claude Code has no native ACP (adapter is a "compatibility boundary; check release notes when Claude settings or auth change"), "thinking transparency depends on agent implementation", permission granularity varies, resume must be attempted explicitly. OpenClaw audit (above): thin notifications, missing replay, dropped MCP, no permission flow. General: `rawInput`/`rawOutput` optional, no model-id/system-prompt fields, subagent sessions still a draft RFD requiring vendor-specific `_meta` capability advertisement. **V**

---

## 4. Answers to the research questions

1. **Protocols/stability.** Native protocols are vendor-owned and versioned for Claude Code (per-field version gates), Codex (schema generators, experimental-capability gate), OpenCode (OpenAPI), OpenClaw (WS v4); Hermes and Cursor document theirs but without stability promises; Gemini's is Google-owned. ACP v1 is stable at the protocol level, but for Claude Code and Codex the ACP path runs through Zed/community adapters that wrap the vendor SDK/app-server, adding a second moving part.
2. **Completeness.** Claude Code stream-json and Codex app-server are the only two that expose essentially everything the harness itself records (thinking, full tool I/O as sent to the model, usage, model, compaction markers, subagents, permission events, hooks). OpenCode SSE is equivalent to the TUI's data source. ACP loses per-turn model id and system prompt for every agent, makes raw tool I/O optional, and depends on adapters for subagents/hooks.
3. **Behaviour change.** Claude Code `-p` without `--bare` equals interactive (Anthropic states this explicitly) but `--bare` may become the default; Codex/OpenCode/OpenClaw/Hermes TUIs are themselves clients of the same server, so no change; Cursor and Gemini headless modes suppress thinking or require `--force`; ACP adapters for Claude Code drop hooks and agent teams.
4. **Input chokepoint.** Only Claude Code `-p` and a tracker-spawned Codex app-server over stdio (not the daemon) give a genuine single stdin path; OpenCode and OpenClaw are multi-client servers by design; Hermes shares a DB; Cursor/Gemini headless have no documented second path but also no documented guarantee.
5. **Resume.** All seven support resume by id; ACP now has both `session/load` (replay) and `session/resume` (no replay).
6. **Subscription auth.** Claude Max works with `claude -p` today and still draws from plan limits (credit-pool split paused); using a Claude subscription inside any non-Anthropic process (ACP adapter, OpenCode, OpenClaw, Hermes) is against Anthropic's terms and/or server-blocked. ChatGPT plans work in Codex app-server; Cursor and Google logins work headless.
7. **Ecosystem.** See section 3.
