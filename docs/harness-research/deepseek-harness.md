# DeepSeek Harness — identification and run-capture research

> **Pre-spike research, superseded.** Written 2026-09-02 from documentation and source reading, before the M0 capture spikes. `docs/CAPTURE-RESEARCH.md` is the measurement and `DECISIONS.md` D23 is the tier table; read `docs/CAPTURE-RESEARCH.md` section 8 — the errata for these files — before trusting anything here.

## Identification

**What "DeepSeek Harness" is** — **Confidence: High**

| Item | Value | Source |
|---|---|---|
| Official name | **DeepSeek Harness**, CLI binary `dsh` | https://github.com/deepseek-ai/deepseek-harness ; https://deepseek.com/harness/en/ |
| Publisher | DeepSeek AI (org `deepseek-ai` on GitHub) | same |
| Release | Developer preview announced **13 Aug 2026**, alongside V4-Pro-0813 GA on the API | https://venturebeat.com/technology/deepseek-harness-launches-as-open-source-rival-to-claude-code-alongside-v4-pro-on-api-with-higher-prices ; https://thenewstack.io/deepseek-harness-open-source-plugins/ (dated Aug 13 2026) |
| Current version | GitHub releases: v0.1.0-rc.7 (Aug 17) … v0.1.1-rc.2 (Aug 21) … **v0.1.2-alpha.5 (Sep 2 2026)**. Release cadence is roughly daily; "breaking changes expected". | https://github.com/deepseek-ai/deepseek-harness/releases |
| License | MIT (+ THIRD_PARTY_NOTICES.md) | https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/README.md |
| Language | TypeScript/Node (ESM, pnpm monorepo, Node ^22.19 or ≥24); a Python SDK wraps the Node runtime; a small native Node addon (Landlock) | https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/AGENTS.md |
| Install | `npx @deepseek-ai/dsh web` or `npm install -g @deepseek-ai/dsh`; from source `pnpm install && pnpm run build && pnpm dsh web`; Python: `pip install deepseek-harness-sdk` | README above; https://deepseek-harness.github.io/deepseek-harness/en/guide/python-sdk |
| Docs | https://deepseek-harness.github.io/deepseek-harness/ (VitePress projection of `docs/`) | README |
| Architecture | Built on **Cordis** ("everything is a plugin": model adapter, tool registry, session log, agent loop are all plugins); npm packages are `@deepseek-ai/dsh-<name>`, 48 packages | https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/docs/architecture.md ; https://github.com/deepseek-ai/deepseek-harness/tree/master/packages |
| Default models | `deepseek-v4-flash`, `deepseek-v4-pro` via `@deepseek-ai/dsh-llm-deepseek`; other providers (Anthropic/OpenAI/custom) pluggable | https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/docs/config-catalog.md ; https://www.developersdigest.tech/blog/deepseek-harness-dsh-first-look |

**Other candidates considered and rejected** (all third-party, none is "the DeepSeek harness"):
- Deep Code (`@vegamo/deepcode-cli`, github.com/lessweb/deepcode-cli) — third-party, listed in DeepSeek's integration docs but not DeepSeek's: https://api-docs.deepseek.com/quick_start/agent_integrations/deepcode/
- DeepSeek-TUI (Rust), deepseekcode/`dsc` (Go), Reasonix, and ~8 `deepseek-cli` repos by individuals — all third-party: https://github.com/deepseek-ai/awesome-deepseek-agent
- No product named "DeepSeek Code" shipped; "Code Harness / DeepSeek Code" was the hiring-stage working name of the team that shipped DeepSeek Harness: https://www.verdent.ai/guides/deepseek-coding-plan-2026

**Critical auth finding (High):** DeepSeek's own provider in dsh is **API-key only** (`DEEPSEEK_API_KEY` / `.credentials.yaml`); DeepSeek sells no subscription plan — the API is pay-per-token from a topped-up balance (https://api-docs.deepseek.com/quick_start/pricing ; config-catalog above). "Subscription auth" inside dsh exists only through **third-party OAuth plugins** that log into ChatGPT/Codex, Claude, Grok, Kimi, Copilot subscriptions (e.g. https://github.com/V1ki/dsh-plugin-subscriptions , https://github.com/ziyou979/dsh-llm-oauth , https://github.com/deepseek-ai/deepseek-harness/discussions/3427). Organizers should decide whether a "DeepSeek Harness" entry means dsh + DeepSeek API key, or dsh + some other vendor's subscription via plugin.

---

## 1. On-disk session/transcript storage — **Confidence: High** (layout), **Medium** (exact filename suffix)

- **Root:** `$DSH_HOME` (default `~/.dsh`). Session backend `@deepseek-ai/dsh-session-persistence-jsonl`, config key `root` (required). https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/docs/config-catalog.md
- **Layout:** `<root>/--<normalized-cwd>--/<encoded-session-id>/session.jsonl.zstd` (default) or `session.jsonl` when `compression: 'none'`. Package README: https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/session/session-persistence-jsonl/README.md . A third-party walkthrough shows `~/.dsh/sessions/--workspace-name--/session-[uuid]/session.jsonl.zstd` in practice: https://computingforgeeks.com/deepseek-harness-dsh-getting-started/
- **Format:** first logical line = private v0 `SessionHeader` (`version, id, createdAt, cwd?, parentSession?, isSeeded, origin?: 'subagent', delegationDepth?, agentPreset?`), then one JSON line per `SessionEvent` `{type, seq, time, data, ignorable?, sourceEventSeqs?, surfaceOp?}`. Default physical encoding: one checksummed Zstandard frame for the header + one checksummed frame per append batch; `packChunks: true` packs consecutive `assistant/chunk` deltas into single rows. Types: https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/core/session/src/types.ts ; catalog: https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/docs/persistence-catalog.md
- **Completeness — governing invariant:** "Model-visible means logged"; `packages/core/agent-loop/src/invariant.ts` requires the outgoing request's messages to byte-match `session.deriveMessages()` (architecture.md; developersdigest link above). What is in the log:
  - **System prompt + tool schemas + call config:** `request/header` `{header: EpochHeader{config, adapterDefaults?, system?, tools?}, reason: 'initial'|'resume'|'change'|'series'}` (types.ts). Re-emitted on change.
  - **Provider/model:** `request/context {provider, model, contextWindow?}` on route/capacity change; `model/selection`; and every `assistant/message.message.source` is `{kind:'model', provider, model}` (https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/llm/llm/src/message.ts).
  - **Reasoning:** `ReasoningBlock` inside `assistant/message.content`; raw `assistant/chunk` (text/reasoning/tool-call deltas) also persisted losslessly (https://deepseek-harness.github.io/deepseek-harness/en/reference/subsystems/llm-streaming ; design note https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/.agents/notes/implemented/architecture/2026-06-14-session-persistence.md).
  - **Tool I/O:** `tool/call {callId, name, arguments(raw JSON string)}` and `tool/result {message: ToolResultMessage, error?, meta?}` (surface event → exactly what the model saw).
  - **Loaded context files / skills / hook injections:** appear as `user/message` with `source {kind:'plugin', plugin:'<name>'}` (https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/core/agent/README.md).
  - **Approvals, sandbox, permission, plan mode, compaction, hooks, goals, todo, subagent descriptors, team messages:** ~70 log-only event types enumerated in persistence-catalog.md.
  - **Caveat — spill:** tool outputs above `maxInlineBytes` are written to a spill store (`<root>/session-<sha256(sessionId)>/<random>-<safeName>`, dir 0700, files 0600) and the log keeps only head/tail preview + `SpillRef`. The model also sees only the preview + ref, so the log still equals model-visible text, but a full-fidelity archive must also capture the spill directory. https://deepseek-harness.github.io/deepseek-harness/en/reference/subsystems/spill
- **Append-only vs rewritten:** "Committed events are never rewritten"; `create()` writes nothing until first append; crash → torn tail discarded and a synthetic `turn/end {reason:{kind:'interrupted'}}` appended; no deletion API; one live writer per session, in-process only (jsonl README; https://deepseek-harness.github.io/deepseek-harness/en/reference/subsystems/persistence). Compaction uses `surfaceOp: {op:'replace', start, end}` to shadow history without deleting it (docs/subsystems/session.md).
- **Subagents:** each child is its own durable Session/file; `SessionHeader.parentSession`, `delegationDepth`, `origin:'subagent'`; `subagent/descriptor` appended once before the child's first request; fork-based children seed with the parent's completed-turn prefix (`isSeeded`, `session/end-seed`). **External** subagent providers (`dsh-subagent-claude-code`, `-codex`, `-acp`) log only the delegation call and final result — their internals are not captured. https://deepseek-harness.github.io/deepseek-harness/en/reference/subsystems/subagent
- **SQLite backend:** removed in v0.1.2-alpha.3 (Aug 31) — JSONL is the only first-party backend now. https://github.com/deepseek-ai/deepseek-harness/releases
- **Derived index:** `@deepseek-ai/dsh-session-query-sqlite` builds a separate FTS index (`path`, WAL) — derived, not authoritative (config-catalog).

## 2. Hooks / plugin / event API — **Confidence: High**

- **Native extension surface = Cordis plugins** (TS module exporting `apply(ctx)`), mounted via profile bundles/`cordis.patch.yml` under `$DSH_HOME/profiles/<name>`; installed with `dsh plugin --profile <name> add <pkg>`. https://deepseek-harness.github.io/deepseek-harness/en/develop/basic/ ; https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/apps/cli/README.md
- **Events available to a plugin** (architecture.md, agent-lifecycle, tool pipeline):
  - `session/event` — broadcast of every durable SessionEvent (full payloads; this is the complete transcript stream).
  - Agent (live, waterfall): `agent/pre-step` (may rewrite/reject messages), `agent/request`, `step/start`, `step/end`, `agent/turn-stopping`.
  - LLM: `llm/stream` waterfall around every model call (full `GenerateOptions`: provider, model, messages, system, tools, sampling, sessionId, purpose) — https://deepseek-harness.github.io/deepseek-harness/en/reference/subsystems/llm-streaming
  - Tools: `tool/call → tools/pre-execute (allow/ask/deny) → tools/execute → tools/post-execute (may rewrite result) → tool/result` — https://agenticcontrolplane.com/controls/dsh
  - `telemetry/*`, `fs/*`, `session-telemetry/record`.
- **Claude Code / Codex hook bridges** (`@deepseek-ai/dsh-hooks-claude-code`, `-codex`, shared `hook-protocol`): 7 Claude events supported (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop, SubagentStart, SubagentStop). Payload: `session_id, transcript_path (always empty string), cwd, hook_event_name, prompt / tool_name / tool_input / tool_response / stop_reason`. Config path via `configPath` (a `hooks.json` or settings file), **read once at startup, no live reload**. Codex bridge stamps a configured `model` string (not the resolved model). https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/hooks/hooks-claude-code/README.md ; config-catalog. Every hook invocation is itself logged (`hook/invoked`, `hook/result`).
- **Can a participant edit config mid-run?** Profile `package.json`/`cordis.patch.yml` changes require a restart in stock dsh (third-party `dsh-hotswap` exists precisely to add hot enable/disable/mount from the Web GUI: https://github.com/HongzhongL/dsh-hotswap). But `settings.yaml` and `.credentials.yaml` are **hot-reloaded by default** (`watch: true`), and the Web UI lets the user switch model without restart (config-catalog; https://www.datacamp.com/tutorial/deepseek-harness). Model switches land in the log as `request/context`/`request/header{reason:'change'}`.

## 3. Telemetry / OpenTelemetry — **Confidence: High**

- First-party `@deepseek-ai/dsh-session-telemetry-otel` (OTLP/HTTP **logs**, not traces) mounted in every base-backed profile except `sdk-minimal`. Modes via `DSH_TELEMETRY_MODE`: `FULL` | `FEEDBACK_ONLY` (default: uploads only when user runs `/feedback`) | `DISABLED`. Default endpoint `https://harness-telemetry.deepseeksvc.com/v1/logs`, override `DSH_TELEMETRY_OTLP_URL` or config `exporter.url`. Hard off: any non-empty `DSH_TELEMETRY_DISABLED`. Exports are the **raw** session mirror (full messages, tool args/results, system prompt, cwd) with **no redaction by default**. https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/.agents/notes/implemented/feature/2026-07-31-web-telemetry-default-mount.md ; https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/.agents/notes/implemented/feature/2026-08-10-telemetry-default-off.md ; https://deepseek-harness.github.io/deepseek-harness/en/reference/subsystems/session-telemetry
- Ledger channel mirrors all event types except surplus `assistant/chunk` (only the first chunk per step is kept) — so OTEL is *not* byte-complete for streaming deltas.
- `@deepseek-ai/dsh-session-log-deepseek` uploads incremental log suffixes to DeepSeek's LLM API as request metadata; **`enabled: false` by default**. https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/session/session-log-deepseek/README.md
- Third-party OTLP **trace** exporters: https://github.com/loongsuite/dsh-plugin , https://github.com/linyp/dsh-plugin-langfuse , https://github.com/PerryLink/dsh-observe ; SigNoz docs https://signoz.io/docs/deepseek-harness-observability/
- Privacy implication for the hackathon: set `DSH_TELEMETRY_DISABLED=1` so nothing leaves the box; point a custom OTEL backend at the tracker only if you want a live mirror.

## 4. Headless / server modes — **Confidence: High**

| Mode | Command | Output |
|---|---|---|
| Headless one-shot | `dsh --profile headless "job"` | "Run one fresh persisted session, print the final answer, and exit" — **final answer only** to stdout, session still written to `$DSH_HOME`. Approval "asks" auto-deny (fail-closed). |
| SDK server | `dsh --profile sdk` / `sdk-minimal` | newline-delimited JSON-RPC over stdio; clients "observe session events, agent status transitions, and subagent completions". Python `RunResult(session_id, final_response, finish_reason, events, notifications)`; `notifications` include descendant subagent sessions in wire order. |
| ACP server | `dsh --profile acp` | Agent Client Protocol over stdio (editor clients e.g. Zed); create/list/resume/close sessions, semantic updates, permission prompts. |
| Web | `dsh web` (127.0.0.1:3080) | Browser UI; API is `POST /api/<ns>/<method>` RPC plus a separate multiplexed frame stream to the browser; no documented public subscription endpoint for third parties. |

Sources: https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/apps/cli/README.md ; https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/sdk/README.md ; https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/acp/README.md ; https://deepseek-harness.github.io/deepseek-harness/en/guide/python-sdk ; https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/docs/api-gateway.md ; https://agenticcontrolplane.com/controls/dsh

## 5. Model identity — **Confidence: High**

- Per assistant message: `assistant/message.data.message.source = {kind:'model', provider, model}` (message.ts). Per request: `request/context {provider, model, contextWindow?}` and `request/header` (EpochHeader `config: LlmCallConfig`). Selection intent: `model/selection`, `agent-preset/selected`. Subagents: own session with `subagent/model-selection-policy {allowedModels}` and their own `request/context`. Compaction summaries carry `provider, model` too (persistence-catalog). Mid-run switches produce new `request/context` + `request/header{reason:'change'}`.
- Note: the "model" string is the provider's id; via third-party OAuth provider plugins it will be whatever that plugin reports.

## 6. Human vs harness-authored messages — **Confidence: High** (attribution), **Medium** (channel)

- Every message carries `source: MessageSource` — `{kind:'user'}` | `{kind:'plugin', plugin}` | `{kind:'model', provider, model}` | `{kind:'tool', callId}` (message.ts; agent README). Docs: `user/message` sources are "direct human prompt", `agent.inject()` (files, skills, notifications), goal continuation, hook-bridge context — the latter three are `plugin`-sourced. https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/docs/subsystems/session.md
- So: a **typed human message = `user/message` whose `data.message.source.kind === 'user'`**. Anything else the model sees as user-role is plugin/tool attributed. Each message has exactly one source. Also `agent/inbox/spliced` durably records inbox insertions/removals (incl. `agent/pre-step` rewrites), so a plugin that rewrote a human prompt leaves a trace.
- **Limitation:** `kind:'user'` does not say *which channel* (Web UI, SDK `followup()`/`steer()`, ACP, headless argv). SDK/ACP clients submit prompts programmatically with the same `user` kind, and I found no field distinguishing steer-during-turn from a fresh turn prompt beyond turn/step position. `feedback/record` (`/feedback` text) never enters model context. `command/run` logs slash commands with a `source`.
- Proving "only `keep going`": filter `user/message` with `source.kind=='user'`; after the first, every one must equal the literal phrase. `plugin`-sourced injections should be whitelisted by plugin name from the frozen profile.

## 7. Third-party recording/export tools — **Confidence: Medium** (existence verified via listings, not audited)

- `zoahdev/dsh-replay` — read-only decoder of `session.jsonl.zstd` (handles packed chunk rows), HTML timeline, diff, JSON stats; MIT: https://github.com/zoahdev/dsh-replay
- `dsh-trajectory` — self-contained HTML trajectory "with a SHA-256 audit stamp"; `plugin-session-export` (Markdown/HTML by source); `dsh-trajectory-debug` (replay, breakpoints, edit-and-rerun); `dsh-event-auditor`; `dsh-audit-bundle` ("content-addressed audit indexes across independent evidence producers", https://github.com/dongsheng123132/dsh-audit-bundle); `dsh-report-studio` ("verifiable receipts", https://github.com/ciceroyang/dsh-report-studio). Directory: https://github.com/0xsline/awesome-deepseek-harness
- Repair tools that **rewrite** logs: `dsh-session-repair-ui` (auto-backup then rewrite), `dsh-corrupt-session-repair`, `Coprexist/dsh-session-recovery` (rebuilds `session.jsonl.zstd`, renumbers `seq`) — https://github.com/Coprexist/dsh-session-recovery
- OTEL exporters: see §3.

## 8. Launch modes vs local observability — **Confidence: High** for official modes, **Medium** for third-party shells

| Mode | Official? | Writes `$DSH_HOME` session log locally? | Tracker can observe |
|---|---|---|---|
| `dsh web` (browser UI, loopback) | yes | yes | yes (file tail + optional plugin) |
| `dsh --profile headless` | yes | yes | yes |
| `dsh --profile sdk` / Python & TS SDK | yes | yes (SDK passes explicit `dsh_home`) — tracker must know that home | yes if home is fixed |
| `dsh --profile acp` (Zed etc.) | yes | yes | yes |
| Web UI bound `0.0.0.0` / remote via `dsh-remote`, SSH tunnel, phone apps | config + third-party | yes, on the host running dsh | yes on that host; **remote clients are additional human-input channels** |
| Electron/Tauri desktop shells (`dsh-desktop`, `DSH Deck`, `deepseek-harness-desktop`) | third-party | they spawn the official runtime; home path may be app-specific | yes if the tracker discovers their `DSH_HOME` |
| TUI (`dsh-tui-app`, `Martty`, `dsh-pi-tui`) | third-party plugins/clients | same runtime | yes, same caveat |
| E2B remote runtime | official provider | session log still local; **filesystem/shell effects happen in the cloud sandbox** | log yes, sandbox state no |
| External subagents (Claude Code / Codex / ACP providers) | official providers | only delegation call + result | internals **not** observable |
| DeepSeek cloud-hosted dsh | none found | — | — |

Sources: apps/cli README; awesome list; https://github.com/salathleizhang/deepseek-harness-desktop ; https://github.com/Loki520865/dsh-remote/ ; subagent subsystem page; config-catalog (`host: 0.0.0.0`, `trustedHosts`).

## 9. Tamper surface — **Confidence: High**

- Log is a plain user-owned file: zstd frame checksums detect corruption, **not** tampering (no signatures, no hash chain, no MAC). Anyone can decompress, edit, renumber `seq`, and repack — the community repair tools do exactly this. Persistence has no delete API but the OS does.
- `$DSH_HOME` is participant-controlled: `cordis.patch.yml` and profile bundles can mount a plugin that (a) rewrites messages in `agent/pre-step`, (b) rewrites tool results in `tools/post-execute`, (c) replaces the persistence provider entirely (it is a swappable seam), or (d) injects `plugin`-sourced context. Items (a)/(b)/(d) still leave durable traces (`agent/inbox/spliced`, `hook/*`, plugin-sourced `user/message`) *if* the stock session plugin is running; (c) defeats logging altogether.
- `settings.yaml`/`.credentials.yaml` hot-reload → provider/model swaps mid-run without restart (logged as `request/context`).
- Telemetry exfil is off/feedback-only by default, but a participant could enable FULL and ship traces elsewhere — irrelevant to integrity, relevant to "traces stay private".
- Snapshot/replay: `pnpm run test:snapshot` replays recorded sessions without an API key (AGENTS.md) — proof that logs can be synthesized offline; a plausible-looking log is not proof of a real run.

---

## Recommended capture design (outside-the-process, no proxy)

1. **Pin the environment** (High): organizer-provided launcher sets `DSH_HOME` to a dedicated dir, installs a frozen profile (`--dump-config` hash recorded), `DSH_TELEMETRY_DISABLED=1`, `compression: 'none'` in the jsonl backend so the tracker reads plain JSONL, and `--profile headless` or `sdk` (not `web`) so no second human channel exists. Record dsh version + `git`/npm hash.
2. **Primary evidence = tail the JSONL** (High): tracker `inotify`/`fs.watch`es `<DSH_HOME>/--*--/*/session.jsonl` (and any subagent session dirs by `parentSession`), copies each new line verbatim into an organizer-owned append-only store with a hash chain and per-line receipt timestamps. Also mirror the spill root. Because committed lines are never rewritten, any later divergence between tracker copy and on-disk file is itself evidence of tampering.
3. **Secondary live mirror = one read-only Cordis plugin** (High): organizer plugin subscribes to `session/event` and `llm/stream` and streams to the tracker over a local socket, plus `agent/pre-step` observer to record rewrite attempts. Ships in the frozen profile; its presence is verifiable via `--dump-config`. This also captures pre-flush events if the process is killed.
4. **Verifier** (High): for each `request/header`+`request/context`, recompute `deriveMessages()` semantics from surface events; assert `user/message` with `source.kind=='user'` ⊆ {initial prompt, "keep going"}; assert plugin-sourced injections come only from whitelisted plugin names; assert contiguous `seq`, single `session/end-seed`, and hash-chain continuity.
5. **Optional OTEL** (Medium): point `dsh-session-telemetry-otel` (`FULL`, custom `exporter.url` → localhost collector) at the tracker for redundancy; do not rely on it for chunk completeness.

## Unobservable modes

- External subagent providers (Claude Code / Codex / ACP as children) — internal tokens never enter the dsh log (High).
- Filesystem/shell effects in E2B remote sandboxes — not in the transcript beyond tool I/O (High).
- Any run where the participant swaps the persistence provider or uses a custom `DSH_HOME` outside the tracked dir (High).
- Web UI exposed on `0.0.0.0`/remote shells — observable, but human input from other devices is indistinguishable from local typing (High).
- Third-party OAuth provider plugins: model id fidelity depends on the plugin (Medium).

## Unknowns / could not verify

- Exact wire protocol/auth of the Web host's event stream (MuxFrame over the `/api` connection; cookie `cookieMaxAgeDays`) — whether an external local process can subscribe without being the browser (Low).
- Whether headless mode prints the session id or supports `--resume` (the README example shows `--resume` only for a "tui profile… assuming installed") (Medium).
- Exact `LlmCallConfig` fields inside `EpochHeader` and the `SubagentDescriptorData` schema (imported from `@deepseek-ai/dsh-llm`; not fetched) (Medium).
- Whether steer-during-turn is distinguishable from a fresh prompt by any field other than turn/step position (Low).
- File permissions of session dirs (spill uses 0700/0600; session jsonl not stated) (Low).
- Format stability: v0 "no compatibility implied"; daily alpha releases through 2 Sep 2026 — the tracker must pin one dsh version for the 168-hour window (High that it is unstable; unknown what changes next).
