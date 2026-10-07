# M0 findings — capture spikes, 2026-09-03

> **A dated measurement, not a status.** These are the nine M0 spikes as they ran on 2026-09-03, and they are correct as that. The build has moved a long way since: `STATUS.md` is where it stands, and section 6's ordered open items are maintained as PRD section 9, which is the copy anything else references.

Scope: the nine M0 spike measurements (the historical `spikes/*/REPORT.md` records, with dsh
prep-only). Those report identities and the dsh preparation notes are omitted from the public cut;
they are named below only to identify the historical source of a measured fact, not as files a
reader must have or can rerun. Sources are cited as `spike N` plus assertion or section. This
included document is the dated tracker summary of that measured synthesis. Tier decisions remain
the responsibility of D23; nothing here is applied to
`docs/DECISIONS.md` or `docs/RULES.md`; sections 3–5 are proposals for Balazs.

Matrix consistency check: no two spike edits in `docs/harness-research/README.md`
contradict each other, so nothing was edited. Two stale-but-unedited cells are flagged in
section 6 (the Cursor row predates spike 9; the dsh row names 0.1.2-alpha while the
installed harness is 0.1.1-rc.2).

---

## 1. Status table

| # | Harness | Version tested | Mode | Tier | A-pass / partial / fail | Driven by | Blockers |
|---|---|---|---|---|---|---|---|
| 1 | Claude Desktop | app 1.44121.4, bundled CC **2.1.258** | attach | `elevated_review` | 7 / 3 / 1 | human (GUI) | no `SessionStart`, no resume marker (A5); raw bodies only via app-wide `env`, captured 23 foreign bodies (A9); A10 unpassable (A10) |
| 2 | Codex CLI | **0.153.0** | launch | `full` | 11 / 0 / 0 | human (TUI) | hook trust must be bypassed or managed, else silent capture loss (Unknowns 2); npm wrapper kill ≠ session kill (A5) |
| 3 | Codex desktop app | ChatGPT.app 26.825.31414 → 41651, embedded 0.150.0-alpha.12.2 → 0.151.0-alpha.7.1 | attach | `elevated_review` + mandatory arming | main 3 / 4 / 4; P3 re-scored A1 A2 A3 A7 pass | human (GUI) | no trust UI → zero posts out of the box (Headline); no resume marker (A5); leg 1 97.7 % foreign bytes (A9); self-updated mid-run (Surprise 1) |
| 4 | Claude Code CLI | **2.1.259** | launch | `full` | 10 / 1 / 0 | human (TUI), orchestrator killed pid | `http` `SessionStart` never dispatches (SessionStart proof); `UserPromptSubmit` carries harness prompts (A2) |
| 5 | OpenCode | **1.18.27** | launch (`serve`, HTTP-driven) | `full`, conditional | 11 / 0 / 0 (A2, A3, A7 qualified) | orchestrator (HTTP API) | leg 1 home-wide (A9); block works only by throwing (A7); TUI + permission leg unobserved |
| 6 | dsh | 0.1.1-rc.2 installed (`next` = 0.1.2-rc.1) | **parked, prep only** | none | not run | — | no session ever run; headless has no resume (omitted historical preparation notes, §"Headless resume"); no folder-scoped config exists (omitted historical preparation notes, §Scoping) |
| 7 | Hermes | **0.19.0** (schema v22, 23 hooks) | launch (REPL over stdin) | `elevated_review` | 11 / 0 / 0 (A7 = recorded, not blocked) | orchestrator (stdin pipe) | `plugins.enabled` cannot be folder-scoped (Unknowns 1); `--safe-mode` is a one-flag capture bypass (Unknowns 4); no rejection surface (A7) |
| 8 | OpenClaw | **2026.8.2** | launch (Gateway) | `elevated_review` | 8 / 1 / 2 | orchestrator (`openclaw agent`) | hot-disable in 0.7 s kills leg 2 *and* the input gate (Unknowns 4); subagent transcripts deleted to archive (A4); rowid cursor lossy (A11) |
| 9 | Cursor CLI | **2026.09.02-c22c1a3** | launch (`-p` + PTY probe) | `elevated_review` | 5 / 3 / 3 | orchestrator (`-p`), human-equivalent PTY probe | `beforeSubmitPrompt` does not fire in `-p` — no human witness, no length gate (Unknown 1); writes 3 app-wide paths + Keychain token (A10); `Subagent` tool never gets `postToolUse` (A3) |

Tier summary: **full** = Codex CLI, Claude Code CLI, OpenCode. **elevated_review** =
Claude Desktop, Codex app, Hermes, OpenClaw, Cursor. **unrated** = dsh.

---

## 2. Cross-harness findings that change the design

### 2.1 `SessionStart` never dispatches to a Claude Code `http` hook
Observed: project-scoped `http` `SessionStart` silent while 16 other project `http` hooks
fired (spike 1, "Unknowns answered"); launcher `--settings` `http` `SessionStart` silent
while 15 other events fired, and a project `command` `SessionStart` fired at startup and
on resume in the same run (spike 4, "The SessionStart proof"). Failure is completely
silent — no error record, no `hook_success` attachment. One resume fires it **twice**
(spike 4, A5).
Harnesses: Claude Code CLI + Claude Desktop.
Changes: session birth cannot be hook-derived on Claude; a missing `SessionStart` must not
raise an anomaly there — while on the Codex app it is exactly the required arming proof
(spike 3 P3).
Edit: ship a `type: command` `SessionStart` shim in the folder-scoped settings; derive
start from launch + first transcript record; dedupe by `(session_id, source)`.

### 2.2 Prompt hooks are not human-only witnesses
Observed: Claude Code delivers a subagent-completion `<task-notification>` through
`UserPromptSubmit` (`origin.kind=task-notification`, `promptSource=system`, **no**
`isMeta`) (spike 4, A2). Codex/OpenCode/Hermes fire the same prompt hook for
subagent/delegate prompts: OpenCode's task-tool prompt is an unmarked `role=user` with its
own `chat.message` (spike 5, A2); Hermes' delegate_task prompt gets its own `pre_llm_call`
(spike 7, A2); Codex subagent rollouts replay the parent's human turns verbatim (spike 2,
A2; spike 3, A2). OpenClaw persists a *blocked* message as an unstamped `role:"user"` row
and re-runs an interrupted turn after restart with an `internal_system` user message
(spike 8, A2/A5/A7).
Harnesses: all five with subagents; OpenClaw additionally on restart.
Changes: "every prompt-hook post after T0 must equal `keep going`" fails open, and its
inverse ("every user-slot row is human") fails closed.
Edit: the human-message rule is two-legged everywhere — a hook post counts as human only
when it pairs with a leg-1 record that passes the harness's positive human test *and* the
root/workspace scoping clauses.

### 2.3 Harness-authored user-slot content — catalogue
| Harness | Content that lands in the user slot or the prompt hook |
|---|---|
| Claude Code | `<task-notification>`, `Continue from where you left off.` (`isMeta`), `/model` triple (`<local-command-caveat>` / `<command-name>` / `<local-command-stdout>`), sidechain prompt, `SubagentStop.last_assistant_message` from the **suggestion agent** (5 on desktop, 3 on CLI, `agent_type=""`, transcript path that never exists) (spikes 1 & 4, A2/A4) |
| Codex | `<recommended_plugins>` **with `<environment_context>` concatenated into the same message**, length varies by project (3,735 vs 13,177 chars); `# AGENTS.md instructions for `; replayed parent turns in subagent rollouts (spike 2 A2; spike 3 A2) |
| OpenCode | task-tool subagent prompt (unmarked); `synthetic: true` shell-endpoint message; title-generator transform fire (spike 5, A2 / Unknowns 1) |
| Hermes | delegate_task prompt; `[Cron delivery:` / `[CONTEXT COMPACTION` prefixes; `[Note: model was just switched …]` injected into `api_content` (spike 7, A2 / Surprise 2) |
| OpenClaw | `subagent_announce` (`inter_session`), `main_session_restart_recovery` (`internal_system`), blocked-message notice (`__openclaw.beforeAgentRunBlocked`) (spike 8, A2/A7) |
| Cursor | subagent task prompt in the same `<user_query>` shape; `stop.followup_message` (spike 9, A2) |
Edit: connectors carry an explicit exclusion list, not a single positive marker.

### 2.4 The artifact root must not be an ancestor of the workspace — nor `$HOME`-adjacent
Observed: the desktop agent ran `ls -la ..` on its first tool call and later `cat`-ed the
tracker settings, receiver log, hook stream and raw-body path (spike 1, Surprise 3). A
mis-launched Codex session with cwd = the artifact root `rg --files`-ed it and listed
`baseline/files/auth.json` (spike 2, Surprise 6). Codex app and Cursor agents read the
operator's global skills from `$HOME` on the first tool call (spike 3, Surprise 4;
spike 9, A10).
Edit: artifacts live in a tracker-owned directory unrelated to the workspace path; the
config baseline hashes credential files, never copies them.

### 2.5 Process-tree kill (Codex npm wrapper)
Observed: `kill -9` on `harnesses/node/bin/codex` killed the wrapper only; the native
binary and `codex-code-mode-host` survived, reparented to pid 1 and held the thread-writer
lock; a `codex resume` that cannot take that lock **silently becomes a new thread** with no
parent reference (spike 2, A5 / Surprise 1). A hard kill leaves no `SessionEnd` on Codex
(spike 2, Surprise 8) or Claude Code (spike 4, A5).
Edit: manifest declares a kill strategy (process group / native pid); a new
`thread_source: "user"` session in a tracked home is an incident.

### 2.6 Hooks are read at session/thread start → arm before start, refuse to stamp `started_at`
Observed: Codex app — trust granted mid-session did nothing for the already-open thread
(zero posts on the next `keep going`); the next *new* thread fired 13 posts across 6 events
and blocked an over-length prompt (spike 3, P3 step 4-5). Hermes reads `config.yaml` and
plugin gates at process start (spike 7, Unknowns 1). Codex CLI hooks load per launch
(spike 2, Unknowns 1). dsh reads its hook config once at startup, and an unreadable config
runs no hooks while the agent starts anyway (omitted historical dsh NOTES §"Leg 2").
Edit: D7 gains a hard gate — no `started_at` without the harness's start signal
(`SessionStart` with the workspace `cwd` on Codex app / Codex CLI; `plugin.loaded` on
OpenCode; `goalcup.loaded` on Hermes; the shim's first post on OpenClaw; launch + first
transcript record on Claude).

### 2.7 Hot-disable of hooks → per-harness hook-silence policy
Observed: OpenClaw `plugins.entries.<id>.enabled: false` hot-applied in **0.7 s**, same
pid, killing leg 2 *and* the input gate; the shim's `setInterval` kept ticking, so plugin
liveness is not proof of arming (spike 8, Unknowns 4). Hermes `--safe-mode` /
`HERMES_SAFE_MODE=1` → 0 posts with a full `sessions` row; `--ignore-user-config` is nearly
as good (spike 7, Unknowns 4). Cursor hot-reloads `hooks.json`: one hook deleted mid-chat
stopped firing on the next turn, no warning (spike 9, "Hot-reload"). Codex trust is
per-hook-hash, so editing `hooks.json` invalidates it (spike 3, P3 constraint 3).
Counter-cases where silence is normal: Claude `SessionStart` (2.1), `PermissionRequest` in
auto mode (spikes 1, 4, 5), Codex `SessionStart` firing at the *first prompt*, not at
launch (spike 2, Surprise 7).
Edit: replace the blanket "hook silence is a flag" with a per-connector table of
{always-expected, conditionally-silent, never-silent} events, plus a
"leg 2 silent while leg 1 grows" detector as the universal fallback.

### 2.8 Tailer cursor rules
| Harness | Cursor | Evidence |
|---|---|---|
| OpenClaw | `(session_id, seq)` per session; **rowid is unsafe** — deleting a subagent generation frees rowids that later inserts reuse; 3 rows (incl. the subagent parent link) were missed live | spike 8, A11 |
| OpenCode | read `event` by `rowid`; `seq` is **per-aggregate** (`unique(aggregate_id, seq)`) and must only be used for contiguity checks; a global `seq` cursor would have dropped the child session's 42 rows | spike 5, A11 / Unknowns 3 |
| Codex | `ordinal`, gap-free per rollout file, contiguous across resume | spike 2, A5/A11; spike 3, A5 |
| Claude Code / dsh JSONL | inode + byte offset, append order (timestamps are out of order inside the file) | spike 4, A11 |
| Hermes | `messages.id` AUTOINCREMENT as a true insert cursor; watch for rows mutated after insert | spike 7, A11 |
| Cursor | `store.db` `blobs`+`meta` by rowid; the JSONL is a display artefact | spike 9, Unknown 2 |
Also: a restarted tailer re-reads from offset 0 and produces a second complete chain — not
corruption (spike 2, A11).

### 2.9 Leg-1 stores are home-wide and must be partitioned at capture time
| Harness | Partition key | Contamination measured |
|---|---|---|
| Codex (app) | first `session_meta.cwd` per file | 20 rollout files, 2 ours; **97.7 % foreign bytes**; plus 1,050 `session_index` thread titles (spike 3, A9) |
| Codex (CLI) | first `session_meta` | 95 of 295 rollout lines foreign (spike 2, A9) |
| OpenCode | `session.directory` | 28 of 458 event rows foreign (spike 5, A9) |
| Hermes | `sessions.cwd` | 1 session + 2 messages foreign (spike 7, A9) |
| OpenClaw | `ctx.sessionKey` — `ctx.workspaceDir` reports the **Gateway's** workspace for every session and cannot filter | spike 8, A9 |
| Claude Desktop (raw bodies) | `metadata.user_id.session_id` | 23 of 44 request bodies foreign, incl. another folder (spike 1, A9) |
| Cursor | `chats/<hash>/…/meta.json → cwd` (a real directory split) | foreign chat in the same config dir (spike 9, A9) |
Contrast: Claude Code CLI process-scoped pins leaked **nothing** — a parallel session under
the same `CLAUDE_CONFIG_DIR` produced zero posts and zero body files, and is detectable as
a second slug in `projects/` (spike 4, A9).

### 2.10 Truncation and spill, per harness
| Harness | Fact |
|---|---|
| Codex | truncated **in band** (`Warning: truncated output (original token count: …)`), no spill file; untruncated bytes exist nowhere (spike 2, A3) |
| Codex app | every tool output stored **4×** (`response_item` + `item.stdout`/`aggregated_output`/`formatted_output`); leg 1 hit 23.2 MB in 11 min (spike 3, A3 / Surprise 5) |
| OpenCode | spills to `tool-output/`, and **the spill file is itself capped at 192 KiB**; `tool.execute.after` sees post-truncation output (spike 5, A3 / Unknowns 2) |
| OpenClaw | three divergent copies: 8,014 chars to the model, 64,000 persisted, 199,999 only in leg-2 `after_tool_call.result.details.aggregated`; `result.details` dropped for every tool at persistence (spike 8, A3) |
| Claude Code | `<sid>/tool-results/` spill path **never exercised** in spike 1 or 4 |
| Hermes | `hook_outputs/` spill **never exercised** (spike 7, A3) |
| Cursor | full tool output on leg 2 and in `store.db` blobs; the JSONL has zero `tool_result` blocks (spike 9, Unknown 2) |
Consequence: "every token the agent saw" holds; "every byte the tool produced" does not.
Say so in `RULES.md`.

### 2.11 Isolated homes are not credential or config boundaries
Observed: Hermes seeded a `copilot` credential from the host's `gh auth token`, and the
same code path reads `~/.claude/.credentials.json` and `~/.qwen/oauth_creds.json`; it also
downloaded a 32.8 MB `tirith` binary mid-run (spike 7, Surprise 1 / A10). OpenCode's system
prompt was 57 % an `<available_skills>` block read from the operator's real
`~/.claude/skills` and `~/.agents/skills`, and the agent loaded one mid-run (spike 5, A10).
The Codex app injects `~/.codex/memories/memory_summary.md` plus the global skill listing
into every thread — and in P3 that made the agent **refuse to write the requested file**
(spike 3, Surprise 3 / "The missing hello2.py"). Claude Desktop's `env` block is app-wide
(spike 1, A9). Cursor puts its token in the login Keychain and syncs `skills-cursor/`
(spike 9, A10). Clean counter-examples: `CLAUDE_CONFIG_DIR` gave a genuinely clean tier
(spike 4, Surprise 8) and OpenClaw's skill discovery is isolated (spike 8, Surprise 7).

### 2.12 A10 cannot pass on GUI apps → named allowlist
Observed: Claude Desktop rewrites `claude_desktop_config.json` and `config.json` during
normal use (spike 1, A10). The Codex app writes `[projects.<ws>] trust_level`, removes a
marketplace timestamp and bumps its own version key while self-updating (spike 3, A10). The
Claude Code CLI's A10 "failure" was an unrelated concurrent session touching
`~/.claude.json` 3 minutes before launch (spike 4, A10).
Edit: A10 becomes "no change to a named allowlist of harness-behaviour config files"
(`~/.claude/settings*.json` + managed settings; `~/.codex/{config.toml,hooks.json,
requirements.toml}`; `$HERMES_HOME/config.yaml`; OpenClaw `openclaw.json`; Cursor
`cli-config.json` + `hooks.json`), with the app's own UI/telemetry state out of scope, and
baselines taken immediately before launch with mtimes recorded.

### 2.13 Trailing-newline normalisation
Observed: the Codex desktop app appends `\n` to every composer message (78 bytes vs the
CLI's 77); the hook posts the newline too, so the two Codex legs agree with each other and
only cross-surface comparison needs normalisation (spike 3, A1 and P3 A1). Cursor's JSONL
wraps human turns in `<timestamp>…</timestamp>\n<user_query>…</user_query>` — substring,
not equality (spike 9, A1). OpenClaw's `llm_input.prompt` carries a timestamp prefix;
`before_agent_run.prompt` is the byte-exact field (spike 8, A1). Hermes' `api_content`
legitimately differs from `content` (spike 7, Surprise 2).
Edit: the verifier normalises exactly one trailing newline, per-harness declares which
field is byte-exact, and never treats wire-vs-typed divergence as tamper evidence on its
own.

### 2.14 Model-switch visibility
| Harness | Where the switch shows | Persists app-wide? |
|---|---|---|
| Claude Code CLI | `PreModelSwitch`/`PostModelSwitch`, `attachment/model`, `message.model` | **yes** — `<config-dir>/settings.json` gains `"model": "sonnet"` (spike 4, A6) |
| Claude Desktop | same hooks; blind spot: a Haiku title generator in no record (spike 1, A6) | not observed |
| Codex CLI | `turn_context`, `thread_settings_applied`, `world_state`, every hook payload | **yes** — `config.toml` (spike 2, A6) |
| Codex app | `turn_context` + `thread_settings_applied` + a `<model_switch>` developer message; effective next turn | **no** (spike 3, A6) |
| OpenCode | `providerID`/`modelID` on every assistant *and* user message | per-request override; TUI `/model` untested (spike 5, A6) |
| Hermes | `pre_api_request` per call, `session_model_usage`; `on_session_start` **re-fires** with the new model | no (spike 7, A6 / Surprise 3) |
| OpenClaw | session-scoped pin, effective next turn, via the `session_status` tool | no (spike 8, "Model switch mechanics") |
| Cursor | per-hook `model`; fields non-uniform (full slug vs base, `""` on Subagent) | `meta.lastUsedModel` only (spike 9, A6) |
Universal: **auxiliary model calls invisible to the transcript** — Claude Haiku title
generator ×3 and a full-context suggestion agent (spikes 1 & 4), OpenCode title call
(spike 5), Hermes `task='title_generation'` (spike 7). Four for four; treat as a rule, not
a quirk.

### 2.15 Resume markers
| Harness | Marker |
|---|---|
| Codex CLI | `SessionStart.source = ["startup","resume"]`, same file, gap-free `ordinal` (spike 2, A5) |
| Claude Code CLI | `command` `SessionStart` source=`resume` (fires **twice** for one resume) + synthetic `Continue from where you left off.` (spike 4, A5) |
| Claude Desktop | **none** — no `SessionStart`, no transcript marker, only a timestamp gap (spike 1, A5) |
| Codex app | **none** — same file, gap-free ordinals, only a 4 m 32 s gap (spike 3, A5) |
| OpenCode | **none durable** — the `event` table has 4 types and no boundary; visible only on the SSE bus and via a second `plugin.loaded` (spike 5, A5) |
| Hermes | no `on_session_start` on resume; a new `goalcup.loaded` with no new `sessions` row (spike 7, A5) |
| OpenClaw | none in `session_windows` (`reason` NULL); the Gateway **re-runs the interrupted turn by itself** (spike 8, A5) |
| Cursor | `sessionStart` fires once per chat, never on resume; `sessionEnd` fires per turn (spike 9, A5) |
Consequence: on five of eight, D9's clean-vs-dirty gap classification is the only
instrument, and "silence after a crash" is not safe (OpenClaw self-restarts).

### 2.16 What remains untested
- Spill paths: Claude `<sid>/tool-results/` (spikes 1, 4), Hermes `hook_outputs/` (spike 7).
- Compaction / `/undo` / rewind: never triggered on any harness (spikes 4, 5, 7, 8, 9).
- Permission / approval leg: zero events on Claude Code (auto mode), OpenCode
  (`permission: null`), Hermes (classifier never tripped), Cursor (`--force`).
- Claude Code: kill **mid-stream of a response** (only mid-request was achieved, spike 4).
- Codex app: `SubagentStart`/`SubagentStop` never observed; `SessionStart` on **reopen**;
  `requirements.toml` managed hooks (spike 3).
- Cursor: `beforeSubmitPrompt` in `-p` — measured **absent**; MCP hooks untested;
  `exit2` block route not exercised (spike 9).
- TUI behaviour on OpenCode, Hermes, OpenClaw, Cursor (all orchestrator-driven).
- dsh: everything — no session was ever run.
- Codex: `codex queue` / `fork` / `remote-control`; OpenCode `/undo` and `import`.

---

## 3. Proposed changes to `docs/DECISIONS.md` (not applied)

1. **D2 (two legs).** Add: on Codex app, OpenClaw and Cursor the legs are not merely
   corroborating — reconstruction *requires* both (blocked-prompt text exists only in
   leg 2 on Codex app [spike 3 P3 A7]; untruncated tool output only in leg 2 on OpenClaw
   [spike 8 A3]; the only human-text witness is leg 2 on Cursor [spike 9 A2]). Add that
   leg-to-leg joins may be impossible by tool id: Codex `exec-<uuid>` never equals
   `call_<id>`, and one rollout call can be N hook pairs — join by `turn_id` + order
   [spike 2 A3; spike 3 P3 A3].
2. **D3 (record raw, interpret later) — extend the exception list** from two signals to
   five: transcript stalled while the process lives; hook posts silent while leg 1 grows;
   **arming signal absent at start** [spike 3 P3]; **tracker config hash deviating**
   (Cursor `config_mismatch` proved it works live, spike 9 A11); **harness version changed
   mid-run** (Codex app self-updated, spike 3 Surprise 1).
3. **D5 (folder-scoped only) — record three named app-wide exceptions**, each disclosed on
   the public entry, bracketed with before/after hashes and reversible: Claude Desktop
   `OTEL_LOG_RAW_API_BODIES` in `~/.claude/settings.json` [spike 1]; twelve
   `[hooks.state…] trusted_hash` tables written by Codex's own trust UI into
   `~/.codex/config.toml` [spike 3 P3]; one `plugins.enabled` key in
   `$HERMES_HOME/config.yaml` [spike 7 Unknowns 1]. Add a fourth, involuntary one for
   Cursor: `agent-cli-state.json`, `ai-tracking/`, `skills-cursor/**` and a Keychain token
   are written despite all three pins [spike 9 A10].
4. **D5 — artifact placement.** The artifact root must not be an ancestor of the workspace
   and must not sit where a mis-launched session can enumerate it; config baselines hash
   credential files without copying them [spike 1 Surprise 3; spike 2 Surprise 6].
5. **D5 — "empty workspace" is not "empty context".** Codex app injects the participant's
   `memory_summary.md` + global skills; OpenCode and Cursor read `~/.claude/skills` and
   `~/.agents/skills` into the system prompt. Either allow-and-disclose or require them
   off; note that in spike 3 P3 the injected skill changed the agent's behaviour.
6. **D6 (watched, not locked).** Add: rewriting a watched hook file mid-event **invalidates
   Codex hook trust** (the trust is a hash of the definition) and requires a re-arm
   [spike 3 P3 constraint 3]; on Cursor a hot-reload deletion of one hook is detectable
   only by the pinned-hash flag [spike 9 hot-reload probe]. Add that the workspace does not
   stay as written: OpenCode installs 61 MB / 3,645 files into `.opencode/node_modules`
   [spike 5 Surprise 1], Hermes leaves a `__pycache__` `.pyc` [spike 7 Deviation 3],
   OpenClaw writes `AGENTS.md`/`BOOTSTRAP.md`/`IDENTITY.md`/`SOUL.md`/`USER.md` and the
   agent `git init`-ed the folder [spike 8 Surprise 5].
7. **D7 (start flow) — add a hard arming gate.** `goalcup start` must not stamp
   `started_at` until the harness's declared start signal arrives for the tracked workspace
   (§2.6). State the Codex-app arming step explicitly (`codex -C <ws>`, `/hooks`, trust 12,
   quit, *then* open a new app thread) and that a thread opened before trust runs blind and
   silent [spike 3 P3 steps 4-5].
8. **D7 (length check) — correct the per-harness list.** Blocking verified: Claude Code
   CLI + Desktop [spikes 1/4 A7], Codex CLI [spike 2 A7], Codex app **only after arming**
   [spike 3 P3 A7], OpenClaw (`before_agent_run`, a real input gate, 68 ms, rc=1)
   [spike 8 A7], Cursor **TTY only** [spike 9 A7]. Best-effort: OpenCode (works by throwing
   an undocumented defect; participant sees an opaque 500) [spike 5 A7]. Impossible:
   Hermes — no user-message rejection surface, record-only [spike 7 A7]. Add: a blocked
   prompt still persists somewhere on most harnesses (Claude `system/informational` +
   `paste-cache/` + `history.jsonl`; Codex CLI `history.jsonl`) and is therefore
   recoverable evidence; on the Codex app it exists **only** in leg 2 and leg 1 shows a
   four-record empty turn with `last_agent_message: null`.
9. **D8 (`keep going` is the only human message).** Add the harness-authored exclusion list
   (§2.3) as a normative annex, and state that OpenClaw's Gateway may resume an interrupted
   turn with **no** human message at all, which must not be read as a missing `keep going`
   [spike 8 A5].
10. **D9 (gap policy).** Strengthen: on Claude Desktop, Codex app, OpenCode, Hermes,
    OpenClaw and Cursor a resume leaves no marker, so gap classification is the primary
    instrument, not a fallback (§2.15). Add the Codex "silent degrade to a new thread"
    incident type [spike 2 Surprise 1].
11. **D10 (one harness per entry).** Add the concrete detectors proven in M0: a second
    Claude Code under the same `CLAUDE_CONFIG_DIR` leaves a second slug in `projects/`
    [spike 4 A9]; leg 2 on the Codex app caught a *CLI* session in the tracked workspace
    [spike 3 P3 A9]; OpenClaw needs `ctx.sessionKey`, since `ctx.workspaceDir` always
    reports the Gateway's workspace [spike 8 A9].
12. **D11 (seal and attribution).** Confirm the attribution check works: in spike 3 P3
    every file present was explained by a traced write and the *absent* file was explained
    by read-only tool calls. Add exclusions to the sealed manifest:
    `.opencode/node_modules`, `__pycache__`, OpenClaw's bootstrap files, and any `.git`
    the agent creates.
13. **D12 (versions).** "Do not update your harness" is unenforceable where the harness
    self-updates: the ChatGPT app went 26.825.31414 → 41651 and 0.150.0-alpha.12.2 →
    0.151.0-alpha.7.1 mid-run, with `cli_version` frozen in `session_meta` at thread
    creation [spike 3 Surprise 1]. Sample the harness version on a timer, record a change
    as a run event, flag rather than disqualify. Pin Claude Code **≥ 2.1.259** [spike 4
    tier note] and record `cc_version` from the billing header as the finer witness.
14. **D13 (connector bar) — record the tier per harness** as measured (section 1), and add
    that a `full` tier carries named preconditions (Codex: managed/bypassed hook trust +
    native-binary launch; Claude Code: `command` `SessionStart` shim + two-legged human
    rule; OpenCode: `plugin.loaded` arming + `session.directory` partitioning + SSE leg).
15. **Later list.** Move **receiver authentication** off the later list into v1: an
    unauthenticated `127.0.0.1:7777` accepted a post from another spike's leftover shim
    [spike 5 Surprise 8]. Add to the later list: byte-exact capture for Codex app
    (home-wide, 4× duplication), and a protobuf reader for Cursor's `store.db` Merkle blobs
    [spike 9 Unknown 2].

---

## 4. Proposed `RULES.md` consequences

1. **Banned flags/modes (verified capture bypasses).** Hermes `--safe-mode`,
   `HERMES_SAFE_MODE=1`, `--ignore-user-config` [spike 7 Unknowns 4]; OpenCode `--pure`,
   `OPENCODE_PURE=1`, `OPENCODE_DISABLE_PROJECT_CONFIG=1`, `POST /instance/dispose`,
   `POST /global/dispose`, `--attach`, `share`, `import` [spike 5 tier/proposals]; OpenClaw
   incognito threads, `agent exec`, `--local`, `--profile`/`--dev`, `proxy` [spike 8 tier];
   Codex `--ephemeral`, `--remote ws://`, `queue`, `fork`, `remote-control` [spike 2
   proposal 5]; Cursor `-p`/`--print` for tracked sessions, `--api-key`/`CURSOR_API_KEY`,
   `-e/--endpoint`, `&` cloud handoff, `--worktree`, `AGENT_CLI_CREDENTIAL_STORE=file`
   [spike 9 proposals]; Claude `--no-session-persistence`,
   `CLAUDE_CODE_SKIP_PROMPT_HISTORY`. All cloud/remote modes stay banned.
2. **Codex desktop arming step, participant-facing.** Before the goal thread: run
   `codex -C <workspace>` once, type `/hooks`, trust the 12 entries, quit; then open a
   **new** thread in the app. Disclose the twelve `trusted_hash` tables appended to
   `~/.codex/config.toml`. A thread opened before trust cannot be started [spike 3 P3].
3. **Raw bodies on desktop → "no other Claude session on this machine".** If
   `OTEL_LOG_RAW_API_BODIES` is used, the participant must run no other Claude Code session
   for the duration, and the tracker discards any body whose
   `metadata.user_id.session_id` is not the tracked session. Measured cost of not doing so:
   23 foreign request bodies, 121 files / 57.3 MB swept [spike 1 A9 + "Raw-body hygiene"].
4. **Privacy statement must name what is read.** The tracker reads the day's
   `~/.codex/sessions/` directory to find the tracked thread [spike 3 A9]; it reads
   home-wide SQLite stores on OpenCode, Hermes and OpenClaw and discards foreign rows at
   capture time [§2.9].
5. **Evidence-completeness statement.** "Every token the agent saw" is true; "every byte a
   tool produced" is not — Codex, OpenCode and OpenClaw truncate with the untruncated bytes
   existing nowhere on disk [§2.10].
6. **Pre-injected context must be disclosed** (Codex app memory + global skills; OpenCode /
   Cursor host skill discovery), or turned off for the duration [§2.11].
7. **Isolated homes are not credential boundaries.** Hermes reads `gh auth token`,
   `~/.claude/.credentials.json`, `~/.qwen/oauth_creds.json`; the connector must pin
   provider *and* model and record `hermes auth list` + per-call billing fields
   [spike 7 Surprise 1]. Cursor stores its token in the login Keychain [spike 9 A10].
8. **OpenClaw runtime pin.** Require `agentRuntime.id: "openclaw"` per model and verify
   with `openclaw models status` at arm time (no model call needed) [spike 8 "The runtime
   pin"].
9. **Side channels to name:** `codex-app-tools` (`send_message_to_thread`, `create_thread`,
   `fork_thread`, `handoff_thread`) pinned into every app-server on argv, plus
   `[desktop] followUpQueueMode = "queue"` [spike 3]; OpenCode `POST /session/:id/shell`,
   which executes a real command and is invisible to leg 2 [spike 5 Surprise 3]; Hermes
   gateway/serve/cron; Claude Remote Control/Channels.
10. **Billing/quota leakage note.** Codex stamps `plan_type`, credit balance and quota
    percentage into every `token_count` event — 15 times in a ten-minute session
    [spike 2 Surprise 9]. Raw traces must stay private.

---

## 5. Connector-manifest fields proved necessary beyond PRD 5.4

1. **`arming.signal`** — the event that proves leg 2 is live (`SessionStart` w/ workspace
   `cwd`; `plugin.loaded`; `goalcup.loaded`; first shim post), plus **`arming.timeout`**
   and **`arming.recipe`** for the human step (Codex app `/hooks`). [§2.6]
2. **`start_signal.required: bool`** — whether `started_at` may be stamped without it
   (false on Claude, true on Codex app). [spike 3 P3 / spike 4 SessionStart proof]
3. **`store[].cursor.type`** — `ordinal` | `rowid` | `sqlite_pk` | `session_seq` |
   `inode_offset`, with an explicit `unsafe_cursors` list (`rowid` on OpenClaw,
   global `seq` on OpenCode). [§2.8]
4. **`store[].partition_key`** — `session_meta.cwd` | `session.directory` | `sessions.cwd` |
   `ctx.sessionKey` | `metadata.user_id.session_id` | `meta.json.cwd`, applied **at capture
   time**. [§2.9]
5. **`human_rule.id` + `human_rule.exclusions[]`** — the positive test *and* the ordered
   harness-authored exclusion list (prefixes, provenance kinds, `synthetic`, blocked-notice
   markers), plus required scoping clauses (`parent_id IS NULL`, `_delegate_from IS NULL`,
   `thread_source != subagent`, `subagent_history_start_ordinal` cut). [§2.2, §2.3]
6. **`human_rule.byte_exact_field`** and **`normalisation.trailing_newline`** — which leg-2
   field is byte-exact (`before_agent_run.prompt`, not `llm_input.prompt`) and whether one
   trailing newline is stripped before comparison. [§2.13]
7. **`kill.strategy`** — `process_group` | `native_pid` | `wrapper_unsafe`, plus
   `lock_paths[]` (Codex `thread-writer-locks/`) to detect a degraded resume. [§2.5]
8. **`config_allowlist[]`** — the named app-wide files whose deviation is meaningful, so
   A10 is checkable on GUI harnesses. [§2.12]
9. **`hook_silence_policy`** — per event: `expected` | `conditional(<reason>)` |
   `never_silent`; plus `hot_disable: none|restart|hot(<seconds>)`. [§2.7]
10. **`app_wide_exception`** — declared file, key, expected diff shape, reversal procedure,
    and the public-entry disclosure string. [D5 proposals]
11. **`version.self_updating: bool`** + **`version.sample_interval`** and the field that
    records the version per record (`cli_version` is frozen at thread creation on Codex).
    [spike 3 Surprise 1]
12. **`aux_calls[]`** — known auxiliary model calls to whitelist (title generator,
    suggestion agent) so "every API call accounted for" does not misfire. [§2.14]
    **Removed from schema v0 in the M1a fix wave (2026-09-04):** four rows written across three connectors and no reader anywhere, and M2 freezes the schema. Held absent by `aux_calls_is_gone_from_the_schema_and_from_every_manifest` in `crates/goalcup-protocol/tests/manifest_load.rs`. The requirement below is the measurement that motivated it and stands as written.
13. **`workspace_exclusions[]`** — paths the harness creates that must not enter the sealed
    manifest (`.opencode/node_modules`, `__pycache__`, OpenClaw bootstrap files). [D11]
14. **`archive_sources[]`** — stores where records leave the primary table
    (OpenClaw `session_transcript_archives` + `sessions/*.jsonl.deleted.*.zst`). [spike 8 A4]
15. **`receiver.token`** — per-run token on the hook receive path. [spike 5 Surprise 8]

---

## 6. Open items for M1, ordered

1. **Ship the Claude Code CLI vertical slice against spike 4's fixtures** — `command`
   `SessionStart` shim, two-legged human rule, `(session_id, source)` dedupe, raw-body
   classifier on `cc_prompt_id` / `cc_is_subagent` / `cc_prev_req`.
2. **Daemon capture-time partitioning + per-store cursor abstraction** (§2.8, §2.9). Fix
   the historical stub tailer's rowid cursor before it becomes daemon code (spike 8 A11) and lock the
   sequence counter (spike 9 A11, two duplicate seqs).
3. **Arming-gate state machine**: `armed → start_signal_seen → started_at`, with a refusal
   path and on-screen text (§2.6).
4. **Manifest schema v0** carrying the fifteen fields in section 5, with the seven measured
   connectors filled in.
5. **Receiver authentication** (per-run token) and hook-silence policy tables.
6. **Run spike 6 (dsh) for real** — it is the only unmeasured harness; headless has no
   resume, so the browser surface is the only mode, and A9 cannot pass cleanly (one process
   serves many workspaces). Decide `latest` (0.1.1-rc.2) vs `next` (0.1.2-rc.1) first.
7. **Close the four never-exercised legs**: spill paths (Claude, Hermes), permission /
   approval leg (all), compaction / rewind (all), mid-stream kill on Claude Code.
8. **Codex app conformance gaps**: `SubagentStart`/`SubagentStop`, `SessionStart` on
   reopen, `requirements.toml` managed hooks (the cleaner organiser mechanism).
9. **Cursor decision**: ban `-p` and require the interactive CLI, or drop Cursor to
   observation-only. Also needs a protobuf reader for `store.db` blobs.
10. **TUI passes** for OpenCode, Hermes, OpenClaw and Cursor — every one of those spikes was
    orchestrator-driven and cannot speak to on-screen behaviour for a blocked message.
11. **Update `docs/harness-research/README.md`**: the Cursor row still predates spike 9
    (transcript lossiness, `store.db` completeness, `-p` hook table, isolation failure), and
    the dsh row names 0.1.2-alpha while the installed harness is 0.1.1-rc.2.
12. **Rewrite the per-harness research files** that the omitted historical spike reports found wrong:
    `codex-and-cursor.md` (four errors on 0.153 plus the app trust-vs-dispatch distinction),
    `claude-code.md` §4 and §8, `deepseek-harness.md` §5 (no `model/selection` event).
