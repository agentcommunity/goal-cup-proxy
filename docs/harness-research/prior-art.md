# Prior art: cross-harness, tamper-evident tracing of autonomous coding-agent runs (no proxy)

Research date: 2026-09-02. "Verified" = read on a primary page; "Inferred" = reading between sources; "Search-summary" = from a search snippet not opened.

---

## 0. Headline findings

1. **Every target harness now exposes a hook surface that hands you the full transcript path.** Claude Code, Codex, Cursor, Hermes, OpenClaw and OpenCode all have lifecycle hooks/plugins; Claude Code, Codex and Cursor put `transcript_path` in every hook payload. Nobody has shipped a *single* cross-harness recorder that is (a) hooks-driven, (b) tamper-evident and (c) neutral-format. The closest pieces exist separately: **agentwatch** (cross-harness tailing, read-only, no integrity), **DEPOSE** (hash-chain + Ed25519 + RFC 3161 for Claude Code/Codex, AGPL, 1 star), **Harbor/ATIF** (neutral format with maintained Claude Code/Codex/OpenHands converters).
2. **Hooks are user-disableable in every harness** (`disableAllHooks` in Claude Code, `[features] hooks=false` in Codex, etc.). The integrity story cannot rest on hooks alone; it must combine hook events with the harness's own persisted transcript files, and anchor both externally.
3. **The benchmark world converged on "raw trajectories or it didn't happen"**: SWE-bench requires raw rollouts; Terminal-Bench 2.x made ATIF trajectories mandatory after three documented cheating cases and added an "agent judge". No competition found verifies *absence of human input*; they verify *presence of agent work* and inspect trajectories manually or with an LLM judge.
4. **Tamper-evidence on a participant-controlled machine is a detection problem, not a prevention problem.** Practical design: local hash chain → periodic signed checkpoint uploaded to an organizer service (server ack = witness) → optional RFC 3161 timestamp from a public TSA. Rekor v2 is a real option for anchoring checkpoint hashes but the Rust client is explicitly experimental.

---

## 1. Cross-harness trace recorders and observability tools

### 1a. Harness-native surfaces

| Harness | Hook surface | Common payload | Persisted transcript | Content completeness | Can user disable? |
|---|---|---|---|---|---|
| **Claude Code** | 33 hook events incl. `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStart/Stop`, `PreCompact/PostCompact`, `ConfigChange`, `FileChanged`, `SessionEnd` | `session_id`, `transcript_path`, `cwd`, `permission_mode`; `UserPromptSubmit.prompt`; `PostToolUse.tool_input`+`tool_output` | `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`; format "internal … changes between versions" | Full messages, tool_use/tool_result, usage; transcript "written asynchronously and may lag" | Yes: `disableAllHooks`, `--bare`, `--no-session-persistence`, `CLAUDE_CODE_SKIP_PROMPT_HISTORY`, `cleanupPeriodDays` |
| **Claude Code OTel** | `CLAUDE_CODE_ENABLE_TELEMETRY=1` | Events `user_prompt`, `assistant_response`, `tool_result`, `tool_decision`, `api_request`, `api_request_body`/`api_response_body` | n/a | **Redacted by default**. `OTEL_LOG_USER_PROMPTS=1`, `OTEL_LOG_ASSISTANT_RESPONSES=1`, `OTEL_LOG_TOOL_DETAILS=1`, `OTEL_LOG_TOOL_CONTENT=1`, `OTEL_LOG_RAW_API_BODIES=file:<dir>` (untruncated full request/response bodies on disk; thinking redacted) | Yes (env vars) |
| **Codex CLI** | Hooks GA, enabled by default: `SessionStart/End`, `SubagentStart/Stop`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact/PostCompact`, `UserPromptSubmit`, `Stop` | `session_id`, `transcript_path`, `cwd`, `model`; `PostToolUse.tool_response` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | Full; OTel `[otel]` exporter with `log_user_prompt` | Yes: `[features] hooks=false`; hooks hash-trusted; admins enforce via `requirements.toml` |
| **Cursor** | `hooks.json` (user/project/team/enterprise): `sessionStart/End`, `preToolUse/postToolUse`, `beforeShellExecution/afterShellExecution`, `beforeMCPExecution/afterMCPExecution`, `beforeReadFile/afterFileEdit`, `beforeSubmitPrompt`, `stop`, `afterAgentResponse/afterAgentThought`, `subagentStart/Stop`, `preCompact` | `conversation_id`, `generation_id`, `model`, `workspace_roots`, `transcript_path` | `~/.cursor/projects/<hash>/agent-transcripts/<id>/<id>.jsonl` plus `state.vscdb` | **Thin transcript** (no timestamps, tokens, tool_result). `afterShellExecution.output` and `afterMCPExecution.result_json` carry outputs, so hook capture beats transcript capture | Yes |
| **OpenCode** | TS plugins in `~/.config/opencode/plugins/` or `.opencode/plugins/`: `tool.execute.before/after`, `message.updated`, `message.part.updated`, `session.*`, `file.edited`, `permission.*`, `command.executed` | Plugin gets `client` SDK, `project`, `directory` | `~/.local/share/opencode/storage/{session,message,part,session_diff}/…json` (experimental) | Full content on disk in `part/` | Yes (plugin dir) |
| **Hermes (Nous)** | Python plugins via `ctx.register_hook`; ~26 hooks incl. `pre_tool_call`, `post_tool_call`, `pre_llm_call`, `post_llm_call`, `pre_api_request` ("raw messages/history"), `on_session_start/end`, `subagent_start/stop`; shell hooks; signed outbound webhooks | see left | SQLite `~/.hermes/state.db` (`sessions`, `messages`, FTS). Exports JSONL/HTML/MD and Claude Code JSONL | Full | Yes |
| **OpenClaw** | Plugin hooks: `before_tool_call`, `after_tool_call`, `tool_result_persist`, `before_agent_run`, `agent_end`, `message_received`, `message_sent`, `session_start/end`, `before/after_compaction`, `subagent_spawned/ended`; needs `allowConversationAccess: true` | tool hooks: `toolName`, `params`, `toolCallId`; message hooks: `content`, `senderId` | SQLite `~/.openclaw/agents/<agentId>/agent/openclaw-agent.sqlite`; legacy JSONL under `sessions/` | Full (SQLite) | Yes |

Two side-channel facts for the "no human after prompt" rule (verified):
- Claude Code **Remote Control** (`--rc`) and **Channels** (Telegram/Discord/iMessage → session) let messages enter a running session from a phone or chat app. They still surface as `user` messages in the transcript and fire `UserPromptSubmit`, so they are detectable but must be checked for.
- Treat any `user` message after the first as a violation unless it equals "keep going" and audit `system/init` + hook events for `--resume`/`--continue` (Claude `SessionStart.reason` = `startup|resume|clear|compact|fork`; Codex `SessionStart.source` = `startup|resume|clear`).

### 1b. Existing recorders/observability integrations

| Tool | Harnesses | Capture method | Completeness | OSS / status |
|---|---|---|---|---|
| **agentwatch** (mishanefedov) | Claude Code (full), Codex, Gemini CLI, Cursor (config-level only), OpenClaw, Hermes (reads `~/.hermes/state.db`) | File polling/tailing of native session stores; read-only | Full content, MD/JSON export | MIT, TypeScript, v0.0.3 — https://github.com/mishanefedov/agentwatch . Best existing map of *where each harness writes*. No integrity layer |
| **claude-trace** (badlogic/lemmy) | Claude Code only | In-process API traffic logging | Full raw API bodies | Fragile to CLI updates; superseded by `OTEL_LOG_RAW_API_BODIES=file:` |
| **claude-code-log**, **codex-trace** etc. | Claude Code / Codex | Parse native JSONL | Viewers only | OSS |
| **ccusage** | Claude Code, Codex, OpenCode (beta) | Parse native files | Usage/cost only | OSS, active |
| **Langfuse** | Claude Code (Stop-hook plugin), Codex (Stop-hook plugin reading rollout JSONL), Copilot, Cursor (stop hook + transcript), OpenCode, Kiro, Augment — https://langfuse.com/resources/engineering/coding-agent-tracing | Hooks + transcript read | Prompts, tool IO, tokens; "Session context files are not captured"; "A developer can disable a hook in their local config" | OSS plugins, MIT |
| **Braintrust** | Claude Code | Hooks, no proxy | Prompts, responses, tool IO, subagent spans, git metadata | OSS: https://github.com/braintrustdata/braintrust-coding-agent-plugins |
| **MLflow** | Claude Code (`mlflow autolog claude`), Codex (`notify` hook + rollout JSONL) | Hooks | Prompts, responses, tools, tokens | Apache-2.0 |
| **W&B Weave** | Claude Code | `weave-claude-code` daemon; OTel GenAI span tree | Every turn, tool call, subagent; no redaction | OSS |
| **Hugging Face Agent Traces** | Claude Code, Codex, Pi natively; others via STS-Format | Upload raw JSONL | Viewer only | https://huggingface.co/docs/hub/agent-traces |
| **ai-blame** | Claude Code, Codex | Parses traces + Codex ghost commits → per-line model attribution | Provenance | **Rust**, BSD-3, active — https://github.com/ai4curation/ai-blame |
| **OTel GenAI semconv** | Claude Code (traces beta), Codex (logs+metrics), VS Code Copilot | Spans `invoke_agent`/`chat`/`execute_tool`; content in `gen_ai.input.messages`, `gen_ai.output.messages` | | All `gen_ai.*` still "Development"; moved to semantic-conventions-genai repo June 2026 |

Inference: no existing tool covers all named harnesses with hooks and content capture; agentwatch's path table plus Langfuse's per-harness plugins are the closest reusable code.

---

## 2. Trajectory / transcript interchange formats

| Format | Owner / status | Shape | Converters from Claude Code / Codex? | Fit as neutral target |
|---|---|---|---|---|
| **ATIF v1.8** (Apr 2026, "Active") | Harbor / Laude Institute — https://github.com/harbor-framework/harbor/blob/main/rfcs/0001-trajectory-format.md | Single `trajectory.json`: `schema_version`, `session_id`, `agent{name,version,model_name,tool_definitions}`, `steps[]{step_id, source: system|user|agent, message, timestamp, reasoning_content, tool_calls[], observation{results[]{source_call_id, content, subagent_trajectory_ref}}, metrics}`, `final_metrics`, `subagent_trajectories[]`, `extra` | **Yes**: Harbor ships agents for Claude Code, Codex, OpenHands, Mini-SWE-Agent, Gemini CLI that emit ATIF | **Best neutral target.** Mandatory for Terminal-Bench 2.x submissions; supports subagents, multimodal, per-step metrics, `extra` for hash fields |
| **HF STS-Format** | Hugging Face — https://huggingface.co/docs/hub/session-traces-format | JSONL: header `{type:"session",harness,id,name}` then `{type:"message",message{role, content, reasoningContent, toolCalls[], toolCallId, timestamp, model}}` | Hub natively renders raw Claude Code / Codex / Pi files | Good *presentation* target (free viewer), too thin as canonical record |
| **Pi session format v3** | earendil-works/pi | JSONL tree via `id`/`parentId`; `CompactionEntry`, `BranchSummaryEntry`, `ModelChangeEntry` | Hermes exports to it | Nice tree model; single-vendor |
| **Claude Code JSONL** | Anthropic; "internal … changes between versions" | one object per line | n/a | De facto lingua franca but explicitly unstable |
| **Claude `stream-json`** | Anthropic | `system/init`, `assistant`, `user`, `result`; `parent_tool_use_id` for subagents | n/a | Only in `-p` mode; under-documented |
| **Codex rollout JSONL** | OpenAI | `session_meta`, `response_item`, `event_msg`, `turn_context` | Harbor, Langfuse, MLflow, HF parse it | Vendor |
| **OTel GenAI events** | OTel, all Development-status | `gen_ai.client.inference.operation.details` | Claude Code and Codex emit natively | Right wire format for *live* streaming, wrong archival format while unstable |
| **Inspect `.eval`** (UK AISI) | Binary zip; `ModelEvent`, `ToolEvent`, `SandboxEvent` | | None from coding harnesses | Eval-framework-specific |

Recommendation: **normalize into ATIF**, keep the raw harness files alongside (hash-referenced from ATIF `extra`), and generate STS-Format on demand for the free HF viewer. Confidence: high.

---

## 3. Tamper-evident logging on a participant-controlled machine

What exists:

- **DEPOSE** (Aftermath Technologies) is the closest prior art: hash chain over events, Ed25519 signature over a manifest, RFC 3161 token from FreeTSA, RFC 8785 canonical JSON, TypeScript producer + static Go verifier; ingests Claude Code JSONL, Codex sessions, shell history, git reflog. **AGPL-3.0, 1 star, no server anchoring** — https://github.com/Aftermath-Technologies-Ltd/depose . Reuse the *design*, not the code.
- **decision-passport-core**: hash-linked records, offline verifier, TS+Python, Apache-2.0, "no server acknowledgement or anchoring" — https://github.com/brigalss-a/decision-passport-core
- **libro** (Rust): SHA-256 hash-linked append-only chain, **AGPL-3.0-only** — license problem for a permissive CLI.
- Hermes issue #487 proposed exactly this and was **closed as not planned**.

Anchoring / witnessing options:

| Mechanism | Status | Practicality |
|---|---|---|
| **Organizer-run acknowledgement service** (client uploads signed checkpoint `{seq, root_hash, prev_ack}`; server countersigns) | Nothing off-the-shelf for agent logs; pattern is the tlog "witness" model | **Highest value, cheapest.** Server ack proves the chain state existed at time T and prevents post-hoc rewrite. Build this |
| **RFC 3161 TSA** | Public: `https://timestamp.sigstore.dev/api/v1/timestamp`, `timestamp.githubapp.com`, FreeTSA, DigiCert | Cheap third-party anti-backdating; Rust support thin (`tsp-asn1`, `x509-tsp`) — implement the tiny TSQ/TSR wrapper or shell out to `openssl ts` |
| **Rekor v2** (GA 2025-10-10, accepts `hashedrekord` and `dsse`) | Public, free | Good for anchoring the *final* sealed-bundle hash. `sigstore` Rust crate pre-1.0, Rekor client "experimental"; fallback `cosign`/`rekor-cli` |
| **Run your own tlog** | Tessera v1.0 (Go); Rust: Cloudflare **azul** (`tlog_tiles`, `signed_note`) | Over-engineered for one event |
| **TPM / Secure Enclave** | Apple provides no general key attestation outside MDM | Not practical cross-platform. Skip |

Threat-model note: with only local hash chains the participant can regenerate the whole chain. The defence is *frequent* external commitments (server ack every N seconds/events) so any rewrite has to happen before the next ack and leaves a gap, and the raw harness transcript (written independently by the harness) must hash-match what the tracker committed.

---

## 4. Competitions / evaluations with "no human" rules and how they verify

| Venue | Rule | Proof accepted | Cheating found |
|---|---|---|---|
| **SWE-bench** — https://github.com/swe-bench/experiments/blob/main/checklist.md | pass@1; no test knowledge; no hints | "raw rollout trajectories so that the maintainers can verify"; manual inspection | Paper on leaderboard opacity (arXiv 2506.17208) |
| **Terminal-Bench 2.x / Harbor** — https://www.tbench.ai/news/leaderboard-integrity-update | fixed envs, ≥5 trials/task, public trajectories | **ATIF trajectories mandatory for all passing trials**, automated static checks, "agent judge" | OB-1 modified timeouts and shipped encrypted solutions; QuantFlow included test folders; ForgeCode `curl`ed solutions. Community submissions closed for 2.1 |
| **MLE-bench** (OpenAI) | agent must not hand-write predictions or call external LLMs | GPT-4o inspects agent logs for rule breaks; plagiarism check | — |
| **METR** (Vivaria) | agents run inside METR-controlled containers | Server-side transcripts; manual reading | — |
| **ARC Prize** | Kaggle sandbox, no internet | Reproducible code | Kaggle 2020 Petfinder hidden-code cheat |
| **Anthropic / OpenAI hackathons** | Build-with rules, human-in-the-loop allowed | Demo + repo | — |

Inference: nobody has enforced *"no human input after prompt 1"* cryptographically. Terminal-Bench's "mandatory ATIF + agent judge" is the closest. Verifiable signals for this rule: count and content of `user`-role turns (all must equal "keep going"), `SessionStart.reason`/`source` ≠ `resume` except tracker-approved restarts, no Remote Control/channel connections, no `permission_mode_changed`/`ConfigChange` events mid-run.

---

## 5. Workspace provenance

| Building block | What it gives | Status / notes |
|---|---|---|
| **Claude Code file-history checkpoints** (`~/.claude/file-history/<session>/`, only edits via file tools, **not Bash-modified files**) | Pre-edit snapshots joinable to tool calls | Partial; Bash gap is fatal on its own |
| **Codex ghost commits** (author `Codex Snapshot`, dangling objects in `.git/objects/`) | Full tree snapshots per turn | Codex-only |
| **ai-blame** (Rust, BSD-3) | Per-line attribution from Claude Code/Codex traces + ghost commits | Reusable Rust parsing code for both formats |
| **Tracker-side git snapshotting** (`git add -A && git write-tree` after each `PostToolUse`, record tree hash) | Harness-independent, catches Bash edits, cheap | Nothing off-the-shelf; build it (`git2` or `gix`) |
| **gittuf RSL** | Signed, hash-linked ref-change log | Go binary; heavy for participants |
| **gitsign** (keyless OIDC → Fulcio → Rekor) | Signed commits tied to identity + public log timestamp | Proves *who/when*, not *agent vs human*. Use for the final seal commit |
| **in-toto / SLSA** | Standard attestation envelope (DSSE + in-toto Statement) | Use for the seal (`subject` = repo tree hash, `predicate` = trace root hash + ack chain); uploadable to Rekor v2 as `dsse` |
| **Empty-start proof** | No prior art | Build: record `git rev-list --count`, tree hash of cwd, directory listing hash, harness config hashes at `SessionStart`; first event in chain |

---

## 6. Human side-channel detection in external services

| Service | Actor metadata available |
|---|---|
| **Linear** | OAuth `actor=app` makes mutations "come from the app itself"; `Comment.botActor` set when created with `actor=application`; `createAsUser` shows "User (via Application)". If the agent's MCP token is user-scoped, its writes are indistinguishable from the human's: require participants to install the agent integration with `actor=app` — https://linear.app/developers/oauth-actor-authorization |
| **GitHub** | Org/enterprise audit log has `actor_is_bot`, `programmatic_access_type`, `user_agent`; commit `verification`. Distinguishing works only if the agent uses a distinct GitHub App/bot token |
| **Slack** | Bot messages carry `bot_id`/`bot_profile`; Audit Logs API. Only if the agent posts as a bot user |

Inference: no prior art for "detect a human sneaking instructions in via a ticket". Workable rule is architectural: the tracker records the identities the agent uses (app-actor identities), and the reviewer diff-checks external timelines against traced tool calls (every Linear/GitHub/Slack write must match a traced MCP/CLI call by ID and timestamp).

---

## 7. Rust ecosystem building blocks

| Need | Crate(s) | Maturity |
|---|---|---|
| File tailing | `notify` 8.2.0 + `notify-debouncer-*` | Mature. Pair with polling fallback (agentwatch found polling more reliable for these JSONL writers) |
| JSONL / canonical hashing | `serde_json` (+ RFC 8785 JCS via `serde_jcs` or `json-canon`), `sha2`/`blake3` | Mature |
| Hash chain / signed checkpoints | roll your own (~200 LOC) or `tlog_tiles` + `signed_note` from azul; avoid `libro` (AGPL) | Own implementation is lower risk |
| Signatures | `ed25519-dalek`, `ring` | Mature |
| Encrypted local queue | `redb` 4.2.0 ("Stable and maintained") or `rusqlite`; encryption: `age` 0.12.x | redb + age recommended; `sled` still beta |
| OTLP receiver | `opentelemetry-proto` with `gen-tonic` feature gives server stubs; `opentelemetry-otlp` 0.32.0 for re-export | Implement `LogsService`/`TraceService` over tonic; small |
| MCP server | `rmcp` 3.2.0 (Aug 31 2026), official, Apache-2.0, stdio + Streamable HTTP | Mature |
| RFC 3161 client | `tsp-asn1`/`x509-tsp`/`tsp-http-client 0.1.0` — thin | Write ~150 LOC or shell to `openssl ts` |
| Rekor / Sigstore | `sigstore` 0.10.0 + `sigstore-rekor`: pre-1.0, experimental | Use only for the final seal; keep `cosign` CLI fallback |
| git snapshots | `git2` (libgit2) or `gix` (gitoxide) | Both mature |
| Release/signing | `cargo-dist` 0.32.0, signed releases, GitHub attestations, Windows codesign | Fine for a hackathon CLI |

---

## What to reuse vs build

**Reuse (high confidence)**
1. **ATIF v1.8 as the canonical trace format**, raw harness files kept and hash-referenced from `extra`. Reuse Harbor's Claude Code / Codex parsing logic as reference; emit STS-Format for the free HF viewer.
2. **Harness hooks as the primary event source** per the table in §1a. Copy Langfuse's/Braintrust's MIT plugin scaffolds for the hook side; copy agentwatch's per-harness path table for the file side.
3. **Native full-content dumps where they exist**: Claude Code `OTEL_LOG_RAW_API_BODIES=file:<dir>` (strongest evidence for Claude Code); Codex `[otel]` + rollout JSONL; Hermes/OpenClaw SQLite; OpenCode `storage/`.
4. **Public anchors**: `timestamp.sigstore.dev` (RFC 3161) for periodic timestamps; **Rekor v2 `dsse`** entry of an in-toto Statement for the final seal; `cosign`/`rekor-cli` fallback.
5. **Rust crates**: `notify`, `redb`, `age`, `ed25519-dalek`, `blake3`/`sha2`, `serde_json` + JCS, `git2`/`gix`, `rmcp`, `opentelemetry-proto`, `cargo-dist`.
6. **Design (not code) from DEPOSE**: event hash chain + manifest signature + TSA token + static offline verifier.

**Build (medium-high confidence)**
1. **The acknowledgement service** ("witness"): client posts `{participant, seq, chain_head, tree_hash, ts}` signed with a per-participant key every N events/seconds; server countersigns and stores. The only piece that turns a locally forgeable chain into evidence.
2. **Dual-source reconciliation**: independently hash the harness's own transcript file (tailed) and the hook stream; the seal fails if they disagree. Defeats "disable hooks then replay". Check `SessionStart.reason/source == resume`, Remote Control/channel connections, `permission_mode_changed`, `ConfigChange`.
3. **Workspace snapshotting** independent of the harness: `git write-tree` of the workspace after every `PostToolUse`/`afterShellExecution` (catches Bash edits that Claude Code checkpoints miss), tree hash embedded in the event. Empty-start proof = first chain entry with directory hash + `git rev-list --count` + harness config hashes.
4. **"Keep going" verifier**: rule engine over ATIF `source == user` steps; anything after step 1 that is not the literal token is a flag.
5. **Rust harness connectors** for Hermes, OpenClaw, OpenCode. Keep each a thin adapter: hook payload → ATIF step; file → ATIF step.
6. **External-service actor policy**: require `actor=app` for Linear and bot/App identities for GitHub/Slack when the agent writes there; reviewer cross-checks external timelines against traced calls.

**Do not build / skip (high confidence)**
- Network/API proxying (subscription auth blocks it and `OTEL_LOG_RAW_API_BODIES` gives the same bodies for Claude Code).
- TPM/Secure Enclave attestation.
- Own transparency log server (a signed-ack table in Postgres is sufficient for one event).
- Reliance on Cursor's on-disk transcript for completeness; capture Cursor via hooks and note in rules that Cursor traces are thinner.

**Residual risks to state in the rules**
- All hooks are user-disableable and Claude Code says its JSONL "changes between versions": pin harness versions per connector and treat the ack chain, not the format, as the integrity anchor.
- A participant can run an untraced second agent and paste results in; only the `user`-turn check plus workspace tree hashes per tool call make that visible. State that unexplained workspace deltas are disqualifying.
