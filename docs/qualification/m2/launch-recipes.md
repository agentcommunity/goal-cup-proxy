# T02: measured automatic-goal launch recipes, Codex CLI and Claude Code

Updated: 2026-10-07

> **Historical (2026-09-27).** This is a narrow live measurement on branch `m2/t02-launch`
> (base `19a52ea`), with one fix round the same day. It tests whether the two installed CLIs
> accept an initial goal that is submitted automatically, records how they do it, and records
> what each launch writes outside its scoped homes. It is not recorder proof, it authorizes
> no uploads, and it changes no production code, connector or profile. T14 and T16 should
> use these rows as observations. They are not a profile schema. T24 must repeat them
> through the real product.

## Current implementation boundary (2026-10-07)

This record is evidence for the measured recipes only; it is not current schema, packaging,
release or qualification evidence. Current schema-1 discovery checks the configured, installed,
then source connector roots and validates every immediate child with a regular `harness.toml`.
The source/OSS cut has three pinned profiles (Codex, Claude and Hermes), while private `m2-pilot`
staging selects only Codex and Claude and copies those two directories whole. A source pin does
not prove package inclusion.

Current runs use a fresh `HOME`: Codex and Claude retain isolated shared config homes, with the
checked Keychains link for Claude, while Hermes uses a seeded per-run config home. Control API v5
binds create to the daemon's connector digest. A restarted unfinished capture is status/seal only;
it cannot resume. Default builds exclude V1 execution but still parse the two retained Codex and
Claude compatibility manifests; full removal is post-tag ticket OS-29. The draft v0.5 line remains
a planned source-only candidate pending owner approval, with no released or qualified artifact.

## 1. Verdict

| Row | Result | Evidence |
|---|---|---|
| `codex_goal_delivered_once_without_input` | **PASS** (native leg) with the current recipe. Wire leg NOT RUN (§6) | `runs/cx-r1-pos-trustoverride-125447.json`. The first measurement, before the trust override, is `runs/cx-pos-direct-114657.json`. The 64 KiB repeat is `runs/cx-big64k-114856.json` |
| `claude_goal_delivered_once_without_input` | **PASS** (native leg) with the current recipe. Wire leg NOT RUN (§6) | `runs/cl-r1-pos-final-125419.json`. The first measurement, before scoping, is `runs/cl-pos-direct-120549.json`. The 64 KiB repeat is `runs/cl-big64k-120721.json` |
| Negative: missing, empty, leading dash without `--` | measured on both, see §4 | Codex **accepts an empty goal** and runs a model session. Claude refuses it |
| 64 KiB goal via argv | **safe on macOS** with an allowlisted environment, see §5 | `argmax.json`, both `*-big64k-*` rows |
| App-wide writes by the current recipes | **none attributable to the run**, by before/after listing (§7) | `cl-r1-pos-final-125419`, `cl-r1-neg-missing-final-125419`, `cl-r1-bash-homelink-125352`, `cx-r1-pos-trustoverride-125447`. The first-round recipes **did** write app-wide; the leftovers are listed in §7.4 |
| Proxy/CA live check through a loopback probe | **NOT RUN.** The probe's upstream connections fail on this host today (§6) | Prior 2026-09-24 evidence stands: Codex 0.153.0 is unchanged, Claude has moved from 2.1.281 to 2.1.283 |
| Automatic delivery available | yes for both, so no release blocker for this item | |

In this document, "delivered" means three things. The harness recorded the exact goal bytes
as the single user-authored message in its scoped native store (counting rule in §3.3). The
model then acted on a unique tag that exists only in the goal: the output file name and
content. A process outside the harness checked the output bytes after it exited.
Exactly-once *on the wire* could not be observed today (§6).

## 2. Host, binaries, isolation

| Item | Value |
|---|---|
| Host | macOS 26.6.2 (25G83), Darwin 25.6.0, arm64; `getconf ARG_MAX` = 1048576 |
| Codex | codex-cli **0.153.0**, native binary `harnesses/node/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`, sha256 `a29d9e86eef88cbbd69f97ce8c590b1d0a287c8f77424f5eef226b883d7eaa22` (the same hash as `docs/qualification/2026-09-15-codex-recon.md`). The npm wrapper was not used |
| Codex auth | isolated `CODEX_HOME=harnesses/home/codex`, `auth_mode` chatgpt, `last_refresh` 2026-09-24T09:56:46Z. Only key names and those two values were read. `codex login status` reports "Logged in using ChatGPT" |
| Claude Code | **2.1.283**, `~/.local/bin/claude` → `~/.local/share/claude/versions/2.1.283`, sha256 `d8cb1e5c79684cc12a8bfc813e3a2073406921b6245744b3009be3ab5651d21e` |
| Claude auth | isolated `CLAUDE_CONFIG_DIR=harnesses/home/claude`. `claude auth status` reports loggedIn true, authMethod claude.ai, subscription max. It reports **loggedIn false** when `USER` is absent (setting `LOGNAME` alone does not help), and also when `HOME` is an empty per-run directory. It reports **loggedIn true** again when the per-run `HOME` holds a single symlink, `Library/Keychains` → the user's `~/Library/Keychains`. This is consistent with the V1 manifest's note that the OAuth token lives in the login keychain (`connectors/claude-code-cli/manifest.toml:85-86`). No keychain item was queried directly |
| Models | Codex `gpt-5.6-luna`, reasoning effort `low`. Claude `haiku`, `--effort low` |
| Workspaces | fresh empty directories under the session scratchpad, `…/scratchpad/m2-t02/<run-id>/ws`. None is inside a git repository, and no ancestor holds `AGENTS.md`, `CLAUDE.md` or `.codex`/`.claude` (scanned) |
| Runner | `launch-recipes/tools/measure.py` runs one bounded launch with `subprocess.Popen(argv_vector, cwd=ws, env=explicit_dict, stdin=/dev/null)`. There is no shell. All launches were serialized, one harness at a time |

**The environment is an allowlist and is never inherited.** Each launch got
`HOME USER LOGNAME SHELL TMPDIR PATH=/usr/bin:/bin:/usr/sbin:/sbin` plus the harness variables
listed in each row. The measuring shell was itself a Claude Code session. Its environment
held `ANTHROPIC_BASE_URL`, `CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION` and 23 other
`CLAUDE_*` variables. If inherited, they would redirect the model endpoint or switch transcript
saving off (see `spikes/wire-pty/REPORT.md` §4(g)). No base URL was set in any run.
`harnesses/env.sh` was read and sourced for the runner, and the two isolated home paths
come from it. Its `PATH`, `XDG_*`, `CURSOR_*` and `NODE_COMPILE_CACHE` exports are
deliberately **not** passed to either harness.

**How app-wide writes were checked (fix round 1).** Immediately before and after each
round-1 launch, the runner lists names, mtimes and sizes (never contents) under these roots:
`~/.claude`, `~/.claude.json`, `~/Library/Caches/claude-cli-nodejs`, `~/.local/state/claude`,
`~/.cache/claude`, `~/.local/share/claude`, `~/.config/claude` (absent here),
`~/Library/Application Support/Claude` (names and mtimes only), `~/.codex`, `~/.agents`, and
the regular files directly in `~`. It also polls `~/.local/state/claude/locks` every 50 ms,
because version locks are created and removed within a run. Every created, modified or
deleted path is recorded. A path is attributed to the run when its name contains the run id
or the session id; anything else is recorded as unattributed. The same before/after listing
is taken of the isolated home the run uses. A full listing takes about 3 s (≈115,000 entries).

The host is busy during these windows. Its own Claude Code session and its agents write
transcripts under `~/.claude/projects/<this project>/<parent session>/`. The Claude desktop
app writes under `~/Library/Application Support/Claude`, and the ChatGPT app's
`codex app-server` writes `~/.codex/logs_2.sqlite-wal` and `models_cache.json`. The ChatGPT
app's writes also appeared during a *Claude* launch (`cl-r1-bash-scoped-124628`). A 5-second
idle listing with no launch showed the same kind of churn. Unattributed rows are therefore
host activity unless stated otherwise, but a write with a non-unique name cannot be ruled
out by this method.

## 3. Named evidence rows

### 3.1 `codex_goal_delivered_once_without_input` — PASS

| Field | Value |
|---|---|
| Run | `cx-r1-pos-trustoverride-125447`, started 2026-09-27T12:54:47.718Z (the current recipe) |
| argv (vector) | `[<codex native>, "exec", "--json", "-m", "gpt-5.6-luna", "-c", "model_reasoning_effort=\"low\"", "-c", "approval_policy=\"never\"", "-c", "check_for_update_on_startup=false", "-c", "projects={ \"<ws>\" = { trust_level = \"trusted\" } }", "-s", "workspace-write", "--skip-git-repo-check", "-C", <ws>, "--", <GOAL>]` |
| cwd / stdin | `<ws>` / `/dev/null` |
| env names | `CODEX_HOME HOME LOGNAME PATH SHELL TMPDIR USER`. Values: `CODEX_HOME=<main checkout>/harnesses/home/codex`, `HOME=<run>/home` (**empty per-run directory**, §7.2), `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, `SHELL=/bin/zsh` |
| Continuation settings, from the native `turn_context` | `approval_policy: never`, `approvals_reviewer: user`, `sandbox_policy: workspace-write, network_access: false`, `effort: low`, `collaboration_mode: default`. `codex exec` runs one turn and exits. It has no input channel |
| Configured / observed model | `gpt-5.6-luna` low / `turn_context.model = gpt-5.6-luna`, `effort = low` |
| Goal | 543 bytes UTF-8, 6 lines, sha256 `ac0b2f84…ada0399`. It starts with `-`, has double spaces, a tab, a line with leading and trailing spaces, `naïve`, and no final newline. Full text is in the run JSON |
| Native source | `sessions/2026/09/27/rollout-2026-09-27T19-54-48-01a0e2ee-b0bd-76d1-96f9-c0cea030cff9.jsonl` (relative to `CODEX_HOME`), first seen 1.169 s after spawn |
| Session identity | stdout `thread.started.thread_id` = `session_meta.payload.id` = rollout file suffix = `01a0e2ee-b0bd-76d1-96f9-c0cea030cff9`. `session_meta.cwd` = `<ws>`, `originator: codex_exec`, `source: exec`, `cli_version: 0.153.0` |
| Observed exact goal | exactly **one** primary user message equals the goal (source line 7), plus its echo (line 8), under the rule in §3.3. Line 4 holds the `<recommended_plugins>` and `<environment_context>` user items, which are environment content, not the goal |
| Actual tool calls | line 13 `custom_tool_call exec` → `tools.apply_patch("*** Add File: t02-codex-t125447.txt\n+delivered-t125447")`. Line 20 ran `wc`/`od`. Line 27 ran `truncate -s 17`. So the checked bytes are the result of two tool calls: `apply_patch` created the file with a trailing newline, and `truncate` removed it |
| Output bytes check | after exit: `ws/t02-codex-t125447.txt` = `delivered-t125447`, 17 bytes, **exact**, sha256 `43e708f4…6975b34`, birth time 12:54:56.120Z, after spawn. The workspace was empty before spawn, and the file is its only entry |
| Exit / duration | **0** / 19.917 s. Final agent message `done` |
| Side effects | `$CODEX_HOME/config.toml` sha256 is unchanged (`cc20ba96…`) and no `[projects]` entry was added. There are no attributable app-wide writes (§7) |
| Excerpt | `launch-recipes/excerpts/codex_goal_delivered_once_without_input.rollout-excerpt.jsonl` holds source lines 1, 6, 7, 8, 13 and 16, with base instructions and timezone omitted |

The first-round measurement `cx-pos-direct-114657` (the same recipe without the trust
override) also passed: exit 0 in 20.373 s, exact output. That run wrote a trust entry into
`config.toml` (§7.3).

### 3.2 `claude_goal_delivered_once_without_input` — PASS

| Field | Value |
|---|---|
| Run | `cl-r1-pos-final-125419`, started 2026-09-27T12:54:20.431Z (the current recipe) |
| argv (vector) | `["<home>/.local/bin/claude", "-p", "--model", "haiku", "--effort", "low", "--permission-mode", "acceptEdits", "--permission-prompts", "none", "--session-id", "c7e7f470-8e38-4d41-a91d-ea5e77dcd42d", "--output-format", "json", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}", "--", <GOAL>]` |
| cwd / stdin | `<ws>` / `/dev/null` |
| env names | `CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS CLAUDE_CONFIG_DIR DISABLE_AUTOUPDATER ENABLE_CLAUDEAI_MCP_SERVERS HOME LOGNAME PATH SHELL TMPDIR USER`. Values: `CLAUDE_CONFIG_DIR=<main checkout>/harnesses/home/claude`, `DISABLE_AUTOUPDATER=1`, `ENABLE_CLAUDEAI_MCP_SERVERS=false`, `CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS=1`, `HOME=<run>/home` holding only `Library/Keychains` → the real `~/Library/Keychains` (§7.1), `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, `SHELL=/bin/zsh` |
| Continuation settings | `-p` is one turn, with no input channel after the goal. `--permission-mode acceptEdits` auto-accepts file edits in the working directories. `--permission-prompts none` denies automatically anything that would prompt. The transcript records `permissionMode: acceptEdits` |
| Configured / observed model | `haiku`, `--effort low` / `claude-haiku-4-5-20251001`, from every `assistant.message.model` and from stdout `modelUsage`. Effort is **not observable** in the native store: `perTurnEffort: null` |
| Goal | 544 bytes UTF-8, 6 lines, sha256 `d476b3be…d4650fc`, built the same way as the Codex goal |
| Native source | `projects/-private-tmp-claude-501--Users-team-dev-PROJECTS-AgentCommunity-goal-cup-9758e5ce-5876-46aa-b761-95082645ba2c-scratchpad-m2-t02-cl-r1-pos-final-125419-ws/c7e7f470-8e38-4d41-a91d-ea5e77dcd42d.jsonl` (relative to `CLAUDE_CONFIG_DIR`), first seen 0.834 s after spawn |
| Session identity | pre-assigned with `--session-id` before spawn. Every record has `sessionId` equal to it, `cwd` = `<ws>` and `version` 2.1.283. `entrypoint: sdk-cli`. Stdout JSON `session_id` is equal to it |
| Observed exact goal | exactly **one** primary `user` record equals the goal (line 3, `parentUuid: null`, `promptSource: sdk`), plus its echo (line 1), under the rule in §3.3. Environment arrives as `attachment` records, which are never `user` records |
| Connectors | **0** MCP tools and **0** MCP servers in the transcript: no `mcp_instructions_delta` attachment, and 0 occurrences of `mcp__` or `claude_ai`. The deferred tool list has 18 built-in names. The first-round run `cl-pos-direct-120549` had 159 `mcp__` references and an `mcp_instructions_delta` (§7.1) |
| Actual tool call | line 19 `tool_use Write {file_path: <ws>/t02-claude-f125419.txt, content: "delivered-f125419"}`. Line 20 `tool_result`: "File created successfully …" |
| Output bytes check | `ws/t02-claude-f125419.txt` = `delivered-f125419`, 17 bytes, **exact**, sha256 `281c2534…79110724`, birth time 12:54:26.143Z, after spawn. It is the only entry in `ws` |
| Exit / duration | **0** / 7.322 s. Stdout `subtype: success`, `num_turns: 2`, `stop_reason: end_turn`, `result: done`, `permission_denials: []` |
| Side effects | there are no attributable app-wide writes. Nothing was written under `~/Library/Caches/claude-cli-nodejs` or `~/.local/state/claude`. The per-run `HOME` still held only `Library/Keychains` after exit (§7) |
| Excerpt | `launch-recipes/excerpts/claude_goal_delivered_once_without_input.transcript-excerpt.jsonl` holds source lines 1, 3, 19 and 20 |

The first-round measurement `cl-pos-direct-120549` (real `HOME`, connectors loaded) also
passed: exit 0 in 9.050 s, exact output. It wrote app-wide (§7.4).

**Privacy note for E01.** The Claude transcript also carries a `session_context` attachment
holding `userEmail` and a `credential_org` attachment holding `organizationUuid`, with or
without the fix. Neither is in the committed excerpts. Raw native capture of this store
therefore holds personal data. The organization id also appears in the *names* of
account-synced directories in the isolated home (`plugins/synced/<org>_<id>`). Committed
path lists replace such ids with `<uuid>`.

### 3.3 The counting rule an automated "exactly once" oracle must use

A raw count of exact goal strings in the native file is **2** in every positive run, for
both harnesses, by design. An oracle that asserts a raw total of 1 would reject every good
run, and one that asserts ≥ 1 would miss a duplicated delivery. Count this instead:

- **Codex rollout.**
  - *Primary:* records with `type == "response_item"`, `payload.type == "message"` and
    `payload.role == "user"`, where some `payload.content[].text` equals the goal. There
    must be **exactly 1**.
  - *Echo:* the `event_msg` record with `payload.type == "item_completed"` and
    `payload.item.type == "UserMessage"` carrying the same text is the same item. It must
    appear 0 or 1 times and must not be counted.
  - *Guard:* no other role-`user` `response_item` may **contain** the goal as a substring.
    This catches a `<stdin>` append (§4) or a second delivery. Role-`user` items whose text
    begins with `<environment_context>` or `<recommended_plugins>` are environment content.
- **Claude transcript.**
  - *Primary:* records with `type == "user"` whose `message.content` string (or `text`
    block) equals the goal. There must be **exactly 1**, with `parentUuid: null` and
    `promptSource: "sdk"`.
  - *Echo:* the `queue-operation` record with `operation: "enqueue"` and a `content` equal
    to the goal is the input-queue echo, and must not be counted.
  - `last-prompt` records hold a display summary (newlines flattened, cut to 198 characters
    plus `…`). They never equal the goal and must not be counted.
  - *Guard:* no other `user` record whose content is a string may contain the goal as a
    substring. This catches a piped-stdin suffix (§4).
- In the stdin-pipe runs the primary-equals-goal count is 0, so the rule correctly fails
  them.

## 4. Negative and boundary measurements

The first-round runs used the first-round recipe: real `HOME` and connectors on for Claude;
real `HOME` for the first three Codex runs, then per-run `HOME`; no trust override. Each run
varied one thing at a time, and the workspace was fresh and empty for each one. `cl-r1-*`
rows use the current Claude recipe.

| Run | Variant | Exit | Model call? | Native store | Observation |
|---|---|---|---|---|---|
| `cx-neg-missing-114511` | no goal argument | **1** (0.149 s) | no | none | stderr `Reading prompt from stdin... No prompt provided via stdin.` |
| `cx-neg-empty-114511` | `-- ""` | **0** (27.355 s) | **yes** | rollout created | **Codex accepts an empty goal.** It stores a zero-length user message and runs a turn. The agent read `~/.agents/skills/…/SKILL.md` and asked "What would you like help with?". This run used the real `HOME` (§7.2) |
| `cx-neg-dashnosep-114511` | goal with no `--` | **2** (0.014 s) | no | none | clap: `unexpected argument '-l' found; tip: to pass '-l' as a value, use '-- -l'` |
| `cl-neg-missing-120526` | no goal argument | **1** (1.699 s) | no | none | `Error: Input must be provided either through stdin or as a prompt argument when using --print`. The launch still started the user's connectors and wrote `mcp-logs-*` app-wide (§7.4) |
| `cl-neg-empty-120538` | `-- ""` | **1** (0.732 s) | no | none | the same error. Claude refuses an empty goal |
| `cl-neg-dashnosep-120526` | goal with no `--` | **1** (0.051 s) | no | none | commander: `unknown option '<entire goal text>'`. The goal is echoed to stderr |
| `cl-r1-neg-missing-final-125419` | no goal argument, **current recipe** | **1** (0.824 s) | no | none | the same error. **No** app-wide write attributable to the run, and the per-run `HOME` was unchanged |
| `cx-stdinpipe-114743` | goal in argv, stdin a **pipe** | 0 | yes | rollout | The delivered user message is the goal followed by `\n\n<stdin>\nT02-STDIN-SENTINEL …\n</stdin>`. The goal is **not** delivered exactly |
| `cl-stdinpipe-120622` | goal in argv, stdin a **pipe** | 0 | yes | transcript | The delivered message is the goal followed by `\nT02-STDIN-SENTINEL …\n`. The goal is **not** delivered exactly |
| `cx-approval-114833` | goal asks for a write to a sibling directory of `ws` | 0 (13.765 s) | yes | rollout | `apply_patch` is rejected with "patch rejected: writing outside of the project; rejected by user approval settings". The model replied `blocked`. The process did not wait, and the outside directory stayed empty. That directory was under `/private/tmp`. Codex's `workspace-write` sandbox also lets shell commands write there (`sandbox_policy.exclude_slash_tmp: false`). Only `apply_patch`'s own project check was exercised, so this does not show that every outside write is refused |
| `cl-approval-120701` | the same | 0 (11.444 s) | yes | transcript | `Write` is denied automatically. The tool_result is `is_error` with "…this session has no approval surface…". Stdout `permission_denials` lists the Write. The model replied `blocked`. The process did not wait |

Codex also prints `Reading additional input from stdin...` on every run with a goal, because
stdin is not a TTY. With `/dev/null`, nothing was appended: the delivered message was
byte-equal to the goal in every `/dev/null` run.

## 5. The 64 KiB goal bound against argv+environment limits

- `getconf ARG_MAX` = 1048576. The limit was measured by binary search with `execve` of
  `/usr/bin/true`, using each recipe's argv and environment shape with the goal slot varied,
  proxy/CA variables included, and placeholder workspace and CA paths. The kernel accepted a launch **if and only if**
  `Σ(len(argv_i)+1) + Σ(len(env_j)+1) + 8·(argc+envc+2) ≤ 1048576`. This held with zero slack
  for both recipes. Codex: 1328 bytes without the goal, largest goal 1047248. Claude: 1054
  bytes, largest goal 1047522. Both sums are 1048576 (`launch-recipes/argmax.json`). The
  image path was `/usr/bin/true`, so the path's own share was not measured separately.
- Real launches with a goal of exactly **65536 bytes**: Codex used 66611 bytes of exec space
  and Claude 66193, about 6.3% of `ARG_MAX`. Both harnesses stored the whole goal byte-exact
  as one user message and completed the task: `cx-big64k-114856` exit 0 in 12.7 s,
  `cl-big64k-120721` exit 0 in 6.6 s. Neither harness truncates the goal or limits its
  length itself at this size.
- **Verdict:** a 64 KiB goal passed as one argv element is safe on macOS, with about 960 KiB
  of headroom, *provided the environment is the allowlist*. An inherited participant
  environment has no bound and would change the margin.
- **What must refuse before spawn** (T16), independently of any semantic length rule:
  - a goal of more than 65536 bytes (`MAX_GOAL_BYTES`), or invalid UTF-8.
  - a goal containing **NUL**. argv cannot carry it. Python's `subprocess` refuses with
    "embedded null byte", and Rust `std::process::Command` refuses with `InvalidInput`
    "nul byte found in provided data" (measured). Neither creates a child.
  - an **empty or whitespace-only** goal. Claude refuses an empty one itself. Codex does not
    (§4), so the launcher must.
  - a computed exec size above `ARG_MAX`. Rust reports `ArgumentListTooLong` (os error 7)
    at spawn (measured with a 1 MiB argument). Checking before the goal stream is captured
    avoids appending a goal for a launch that cannot happen.
  - always put `--` before the goal element. A goal beginning with `-` is otherwise parsed
    as options (§4).
- Linux (V1.1) has a per-argument `MAX_ARG_STRLEN` of 131072. It was **not measured** here.
- A goal in argv is visible to other local processes through `ps`. Under RULES.md the goal
  becomes public with the entry, so this is recorded rather than treated as a blocker.

## 6. Proxy and CA: status

**Live check through a loopback recording probe: NOT RUN.** The environment blocked it,
for these reasons:

- The only existing probe is the 2026-09-24 `wprobe`. Its 406-line source was reviewed: it
  listens on `127.0.0.1:0` only, MITMs `chatgpt.com`, `api.openai.com` and
  `api.anthropic.com`, and tunnels everything else. It was rebuilt unchanged with
  `cargo build --release --offline --locked` in session scratch: `main.rs` sha256
  `6def6060…`, `Cargo.lock` `c008185d…`, binary `0fc3d047…`. No new proxy code was written.
- Two harness runs through it produced no model exchange. `cx-pos-probe-114919` was stopped
  by the 600 s bound (exit −15). `cl-pos-probe-120740` was stopped by a 180 s bound (exit 143).
  The probe recorded upstream failures. For Codex: 31 × `connect timeout
  [2a06:98c1:3101::6812:202f]:443` and 12 × `tls handshake eof`, with one tunnel to
  `ab.chatgpt.com` completed. For Claude: 30 × `connect timeout [2607:6bc0::10]:443` and
  2 × `[2600:1901:0:9e23::]:443`.
- Self-tests without any harness failed the same way. The probe's `wire.jsonl` records,
  `conntest.c` and `nul.rs` are kept in the ignored out directory; the curl, python and C
  client outputs were not retained. `curl -x` through the probe timed out for `chatgpt.com` (3 of 3), was reset for
  `api.anthropic.com`, and timed out even for a plain `CONNECT` tunnel to `example.com`.
  The original 2026-09-24 binary behaved the same. From the same shell, `python3` opened TCP
  to every address of `chatgpt.com` and `example.com` in 0.01–0.1 s, and completed TLS to
  `chatgpt.com` over IPv4 and IPv6. `curl` completed HTTPS to `chatgpt.com` (IPv4 and IPv6)
  and to `api.anthropic.com`.
  A freshly compiled C connect test timed out on the IPv6 addresses and on some IPv4 ones.
- The cause was not established by this task. The pattern (system and Homebrew binaries
  connect, freshly linked ad-hoc binaries do not) suggests a per-process network filter. The
  orchestrator reports that the host runs an outbound firewall with per-application rules,
  which would explain it; this task did not inspect it. Clearing it is user setup (an allow
  rule for the specific binary), which this task may not do.
- There is one partial observation. In the Codex attempt, the 12 `tls handshake eof` errors
  come from the probe's *upstream* connect, which the probe only reaches after the Codex
  client has completed TLS with the probe's leaf certificate. The probe also recorded 0 client
  TLS failures. So Codex 0.153.0 trusted the process-scoped `CODEX_CA_CERTIFICATE` on 12
  connections. The Claude attempt is **inconclusive** for `NODE_EXTRA_CA_CERTS`: one client
  handshake ended in EOF, and the probe does not record which host the connect timeouts
  were for.
- **Not observed today:** exactly-once goal bytes in the delivered request, an auth host
  tunnelling, and the model endpoints unchanged on the wire.

**Process-scoped variables to use.** These come from the measured 2026-09-24 evidence
(`docs/superpowers/reviews/2026-09-24-proxy-probe.md`, `spikes/wire-pty/REPORT.md` §4(a),
§4(g), §5), and today's runs set them in the same way:

| Harness | Proxy variables | CA variable | Prior version measured | Today |
|---|---|---|---|---|
| Codex | `HTTPS_PROXY`, `https_proxy` = `http://127.0.0.1:<port>`; `NO_PROXY`, `no_proxy` = `127.0.0.1,localhost` | `CODEX_CA_CERTIFICATE=<ca.pem>` was enough on 2026-09-24 (13 decrypted connections, 1 WS upgrade, refresh tunnelled to `auth.openai.com`) | 0.153.0 (same as today) | client-side trust seen, no upstream |
| Claude | same four | `NODE_EXTRA_CA_CERTS=<ca.pem>` was enough on 2026-09-24. Without it: rc 1, "SSL certificate verification failed" | **2.1.281** (today **2.1.283**: unqualified) | inconclusive |

Tool subprocesses: `spikes/wire-pty/REPORT.md` §5 found that `NODE_EXTRA_CA_CERTS` does not
reach `curl`, and recommends `SSL_CERT_FILE`/`CURL_CA_BUNDLE` with a *combined* bundle
(system roots plus the proxy CA). This was not re-measured. In the measured Codex recipe,
`sandbox_policy.network_access` is `false`, so Codex tool commands have no network at all.
The recipe also sets no `HTTP_PROXY`, because the probe refuses non-`CONNECT` requests.

**Behaviour when transport fails**, which matters for E01's stop criteria: neither harness
exits on its own.

- Codex's stdout events show WebSocket reconnects 2/5 to 5/5 ("request timed out"), then
  "Falling back from WebSockets to HTTPS transport", then HTTPS reconnects 1/5 to 3/5. It was
  still running at 600 s.
- Claude wrote `system api_error` records for retries 1–7 of `maxRetries` 10, with backoff
  rising from 0.6 s to 38 s, and was still running at 180 s.
- In both cases the goal was **already recorded in the native store** even though no model
  request completed. A native goal record therefore does not prove that the provider
  received the goal.

## 7. Scope and side effects

### 7.0 Native store scope

| | Codex | Claude |
|---|---|---|
| Store | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<thread_id>.jsonl`. The date directory and filename use **local** time, so glob by thread id and not by UTC date | `$CLAUDE_CONFIG_DIR/projects/<slug(cwd)>/<session-id>.jsonl`. As observed, the slug is the cwd with every non-alphanumeric character mapped to `-`. Truncation of long paths was not exercised, so find the file with the glob `projects/*/<session-id>.jsonl` rather than computing the slug |
| Private or shared | **shared**: every run using this `CODEX_HOME` writes here, including older spike runs | **shared** config directory. The file name is fixed before spawn by the pre-assigned session id |
| Identity known | after spawn: stdout `thread.started` (with `--json`) and the new rollout file (0.12–1.17 s in goal runs) | before spawn, from `--session-id`. The file appears 0.35–2.4 s after spawn |
| Positive scope evidence | the file is new after spawn; `session_meta.id` = thread id = filename suffix; `session_meta.cwd` = ws; `originator codex_exec`; the default store has 0 files with that id | filename = pre-assigned uuid; every record's `sessionId` equals it; `cwd` = ws; `version` 2.1.283; stdout `session_id` equals it; `~/.claude/projects` has 0 files with that id |
| Ownership handoff | the launcher holds the child pid from `Popen`. Session identity is discoverable in under 1.2 s, so binding the process to the session can happen before any receipt | the same, with identity known before spawn |

### 7.1 Claude: connectors, the app-wide cache, the shell snapshot, the version lock

**What the first-round recipe did.** It used the real `HOME` and no MCP settings.

- Every Claude launch that got past argument parsing (7 of 8 first-round runs) started the
  account's claude.ai connectors and the account-synced plugins' MCP servers. That includes
  `cl-neg-missing` and `cl-neg-empty`, which never created a session.
- Each launch wrote a directory named after its workspace slug into the **user's**
  `~/Library/Caches/claude-cli-nodejs/`, holding `mcp-logs-<server>/<time>.jsonl` files.
- The positive transcript carried an `mcp_instructions_delta` attachment and 159 `mcp__`
  tool references (`mcp__claude_ai_<connector>__…`, one prefix per claude.ai connector on the
  account).
- A launch also creates and removes `~/.local/state/claude/locks/2.1.283.lock` while it
  runs. This was observed in the baseline below; the first-round runs were not polled.
- The re-measured baseline `cl-r1-neg-missing-baseline-124416` shows exactly this. The run
  created 21 app-wide entries: its slug directory and 10 `mcp-logs-plugin-engineering-*`
  subdirectories with one file each. The lock poll saw `2.1.283.lock` appear in the user's
  lock directory.
- Where the connectors come from: the `claude-ai-*` servers are the subscription account's
  claude.ai connectors (the bundle's `[claudeai-mcp]` loader). The `plugin-*` servers come
  from account-synced plugins in the isolated home, `plugins/synced/<org>_<id>/`, which are
  refreshed at every launch.
- The cache path: per the bundle's macOS path code, the cache directory is
  `$HOME/Library/Caches/claude-cli-nodejs`. `XDG_CACHE_HOME` is not consulted for it; it
  only sets the updater's `claude/staging` directory. This was not separately measured,
  because with MCP off there were no cache writes to redirect.

**Settings confirmed in the installed bundle, then measured.**

- `ENABLE_CLAUDEAI_MCP_SERVERS=false`: the bundle treats `0/false/no/off` as disabled and
  logs "[claudeai-mcp] Disabled via env var".
- `CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS=1`: "Skipping plugin MCP server discovery".
- `--strict-mcp-config --mcp-config '{"mcpServers":{}}'`.
- These three were measured together. Which one alone suffices for which server class was
  not isolated.

**Shell snapshot.** The first Bash tool call makes Claude run `zsh -c -l <script>`, and the
script contains `source "<homedir()>/.zshrc"`. For zsh, the bundle resolves the file from
`os.homedir()`, not from `ZDOTDIR`. The snapshot is written to
`$CLAUDE_CONFIG_DIR/shell-snapshots/` and deleted at a clean exit. Older snapshots left in the
isolated home are about 140 KB each, with 233 `alias` lines.

| Run | Settings | App-wide writes attributable to the run | MCP in transcript | Shell snapshot (counts only) |
|---|---|---|---|---|
| `cl-r1-neg-missing-baseline-124416` | first-round recipe | **21** (cache slug directory plus `mcp-logs-*`); transient user lock `2.1.283.lock` | (no session) | — |
| `cl-r1-neg-missing-scoped-124428` | MCP off; `ZDOTDIR`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` → per-run dirs; real `HOME` | 0; the lock moved to `<run>/xdg-state/claude/locks` | (no session) | — |
| `cl-r1-pos-scoped-124442` | the same, positive goal | 0; lock per-run | 0 servers, 0 tools | none (Write only) |
| `cl-r1-bash-scoped-124628` | the same, plus `--allowedTools Bash` (measurement only) | 0 by name | 0 / 0 | **139,544 B, 233 alias lines, 232 function definitions**: the user's `~/.zshrc` content is still captured. `ZDOTDIR` only redirected zsh's `.zcompdump` files into the per-run dir |
| `cl-r1-bash-homelink-125352` | MCP off; `HOME=<run>/home` holding only `Library/Keychains` → the real one; `--allowedTools Bash` | 0 | 0 / 0 | **4,038 B, 0 alias lines, 3 function definitions** (Claude's own `find`/`grep`/`pkill` shims) |
| `cl-r1-pos-final-125419` | MCP off; keychain-link `HOME` (**the current recipe**) | **0** | 0 / 0 | none |
| `cl-r1-neg-missing-final-125419` | the same, no goal | **0** | (no session) | — |

(`cl-r1-bash-scoped-124529` is an earlier copy of the scoped Bash run. The runner then only
looked for snapshots after exit, by which time the file was deleted.)

**Conclusions for Claude.**

- MCP off removes the connectors from the model context and removes the cache writes.
- `XDG_STATE_HOME` redirects the version lock.
- **No setting that keeps the real `HOME` stops the Bash tool from sourcing the user's
  `~/.zshrc`.** `ZDOTDIR` was measured and does not help. The same code path would source
  `~/.bashrc` for bash, or `~/.profile` for sh.
- The keychain-link `HOME` stops it, keeps subscription login working, and needs no MCP
  cache or lock redirection, because everything `HOME`-relative lands in the per-run
  directory. In that recipe no version lock was written anywhere.

That is why the current recipe is the keychain-link `HOME`. It has costs the maintainer
should weigh (§9). It hands the harness the real `~/Library/Keychains` directory: the same
access it has under the real `HOME`, but constructed deliberately. It hides the user's
`~/.gitconfig`, `~/.ssh` and other dotfiles from the agent's tools. With no version lock, a
concurrently running Claude updater (for example the host's) is not told that 2.1.283 is in
use; whether it could prune that version mid-run was not measured.

**Still in context with the current recipe.** The `skill_listing` attachment reports 31
skills, the same count as before the fix. Account-synced plugins and skills in the isolated
home are refreshed at each launch (`plugins/synced`, `skills/synced`). The bundle has
`CLAUDE_CODE_SYNC_PLUGINS*` switches, but their effect was not measured.

### 7.2 Codex: `HOME`

With the real `HOME`, Codex 0.153.0 reads user-level skills from `$HOME/.agents/skills`. The
rollout of `cx-neg-empty-114511` lists skill root `r0 = ~/.agents/skills`, and the
agent read `SKILL.md` files there (25 references). `CODEX_HOME` does not prevent this.

That run also rewrote the user's `~/.zcompdump-<host>-<zsh version>` and its `.zwc`. Both
were born 18:45:13 local; the run was spawned at 18:45:12.5. The attribution is by timing:
Codex starts the user's login shell to snapshot it, and the user's `~/.zshrc` runs
`compinit`.

Every later Codex run used `HOME=<run>/home`, an empty per-run directory. With it, those
paths had 0 references, auth still worked (`auth.json` is read from `CODEX_HOME`), and the
per-run home was still empty after each run. The Codex sandbox's `permission_profile` grants
read access to the whole filesystem (`root: read`), so this stops *automatic* loading only,
not deliberate reads by the agent.

### 7.3 Codex: writes into the shared `CODEX_HOME`

- **Trust entry.** Each first-round `codex exec` that created a session (6 of 6) wrote
  `[projects."<ws>"] trust_level = "trusted"` into `$CODEX_HOME/config.toml`. The file now
  holds those 6 entries; they were not removed.
- **Avoiding it.** The process-scoped override
  `-c 'projects={ "<ws>" = { trust_level = "trusted" } }'` avoids the write (run
  `cx-r1-pos-trustoverride-125447`). `config.toml` sha256 was `cc20ba96…` before and after,
  and no project was added. The override replaces the `projects` table *for that process
  only*, so other trusted paths are not visible to it.
- **Other writes into the isolated home (measured in the same run, 51 changed paths):**
  - the session rollout;
  - the SQLite state, log, memory, queue, goal and thread-history databases;
  - `models_cache.json`;
  - `cache/codex_apps_server_info`, `cache/codex_apps_tools` and `cache/remote_plugin_catalog`;
  - `plugins/cache/*/.codex-remote-plugin-install.json` for the account's remote plugins;
  - a per-thread `shell_snapshots/<thread>.sh`, removed at a clean exit;
  - `thread-writer-locks/<thread>.lock` and `tmp/arg0/codex-arg0*`.
- **SIGTERM residue.** The SIGTERM'd run `cx-pos-probe-114919` left its lock, its arg0
  directory and a 1,673-byte shell snapshot behind. The *next* launch deleted the lock and
  the arg0 directory; the snapshot is still there.
- **Connectors.** The developer message contains `<apps_instructions>`, which says apps
  (connectors) are MCP tools that can be lazy-loaded through `tool_search`, and
  `<recommended_plugins>` lists account plugins. Whether any connector tool is actually
  available to the Codex run was not measured.

### 7.4 App-wide writes left by this task (not removed)

Names only. Nothing under the user's app-wide directories was deleted or modified by hand.

- `~/Library/Caches/claude-cli-nodejs/`: 8 directories named
  `-private-tmp-claude-501--…-scratchpad-m2-t02-<run>-ws`, holding 70 files in total, all
  `mcp-logs-<server>/<time>.jsonl`.
  - Runs: `cl-neg-missing-120526` (15 files), `cl-neg-empty-120538` (15),
    `cl-pos-direct-120549` (10), `cl-stdinpipe-120622` (6), `cl-approval-120701` (6),
    `cl-big64k-120721` (6), `cl-pos-probe-120740` (2), and the fix-round baseline
    `cl-r1-neg-missing-baseline-124416` (10).
  - Servers: the account's own `claude-ai-*` connectors and the `plugin-engineering-*`
    servers of its installed plugins.
- `~/.local/state/claude/locks`: the directory mtime changed. The fix-round baseline was
  observed creating and removing `2.1.283.lock` there. The first-round runs were not polled,
  but the directory's mtime (19:10:40) matches the SIGTERM of `cl-pos-probe-120740`. No
  leftover file remains.
- `~/.zcompdump-<host>-<zsh version>` and its `.zwc`: rewritten at 18:45:13 by
  `cx-neg-empty-114511` (by timing, §7.2).
- Reads, not writes: the same Codex run read `~/.agents/skills/**/SKILL.md`. Every
  first-round Claude launch loaded the account's connectors.

## 8. Recipe for E01

> **Preconditions, before any E01 launch and outside the measured run:**
>
> 1. **Network filter.** Freshly built binaries on this host cannot reach upstream (§6); the
>    orchestrator reports the cause as the host's outbound firewall. E01's recording proxy is
>    a freshly built binary too. The user must add an allow rule for that exact proxy binary
>    before E01 starts. Without it, E01 will reproduce §6: no model exchange, and harnesses that
>    retry and never exit. After the rule is in place, the first proxied run per harness is
>    the first live check of these recipes behind a proxy. `CODEX_CA_CERTIFICATE` rests on
>    the 2026-09-24 run plus today's client-side trust, and `NODE_EXTRA_CA_CERTS` on 2.1.283
>    is unqualified.
> 2. **Stdin.** Force the harness child's stdin to `/dev/null`, whatever E01's own stdin is.
>    E01's "direct inherited harness input/output" must not mean an inherited *pipe*,
>    because both harnesses append piped stdin to the goal (§4).
> 3. **Stop procedure.** E01's own hard limit is the ten minutes per smoke from the E01
>    plan. When it expires, send **SIGTERM** to the harness pid, wait a **10 s grace**, then
>    send **SIGKILL**. Afterwards, check for surviving descendants in other process groups
>    (Codex's `codex-code-mode-host` runs in its own group,
>    `docs/qualification/2026-09-15-codex-recon.md` §3) and kill them the same way. In these
>    measurements SIGTERM alone ended both harnesses (Codex −15, Claude 143), and no
>    orphaned helpers remained. Neither harness exits by itself when transport fails (§6).
> 4. **Evidence of side effects.** Record the metadata-only before/after listing of §2 for
>    each E01 run, as `measure.py --appwide` does, and the `$CODEX_HOME/config.toml` sha256
>    before and after.
>
> Copy the argv and environment exactly. E01 sets the "add for capture" variables to its
> own proxy's values.

### Codex CLI 0.153.0

- **Executable:** the native binary
  `<main checkout>/harnesses/node/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`.
  Refuse to launch unless its sha256 is
  `a29d9e86eef88cbbd69f97ce8c590b1d0a287c8f77424f5eef226b883d7eaa22` and `--version` prints
  `codex-cli 0.153.0`. Do not use the npm wrapper, which needs `node` on `PATH` and adds
  `CODEX_MANAGED_*` variables.
- **argv template** (a vector; `<GOAL>` is a single element and always follows `--`):
  `[<codex>, "exec", "--json", "-m", "gpt-5.6-luna", "-c", "model_reasoning_effort=\"low\"", "-c", "approval_policy=\"never\"", "-c", "check_for_update_on_startup=false", "-c", "projects={ \"<ws>\" = { trust_level = \"trusted\" } }", "-s", "workspace-write", "--skip-git-repo-check", "-C", <ws>, "--", <GOAL>]`.
  The `projects` override keeps `exec` from writing a trust entry into the shared
  `config.toml`. It needs `<ws>` to contain no `"` or `\`; T16 must escape it or refuse.
- **cwd:** `<ws>`. **stdin:** `/dev/null`. stdout carries JSONL events and stderr the
  progress text; both can be inherited or captured.
- **Environment:** build it from nothing. Measured: `HOME=<fresh empty per-run dir>`
  (required to keep `~/.agents/skills` and the user's shell startup files out, §7.2),
  `USER`, `LOGNAME`, `SHELL`, `TMPDIR` (the user's values), `PATH=/usr/bin:/bin:/usr/sbin:/sbin`,
  `CODEX_HOME=<main checkout>/harnesses/home/codex`.
  **Add for capture:** `HTTPS_PROXY`, `https_proxy` = `http://127.0.0.1:<port>`;
  `NO_PROXY`, `no_proxy` = `127.0.0.1,localhost`; `CODEX_CA_CERTIFICATE=<proxy CA pem>`.
  Never set `OPENAI_BASE_URL`, `CODEX_BASE_URL` or `OPENAI_API_KEY`.
- **Workspace:** a fresh empty directory outside any git repository, with no ancestor
  `AGENTS.md`. `--skip-git-repo-check` is required there. Tool commands have no network
  (`network_access: false`). The sandbox can also write to `/tmp` and `$TMPDIR`, so keep
  E01's capture storage out of those.
- **Known writes (disclose in E01's evidence):** into the shared `CODEX_HOME`, the list in
  §7.3. A SIGTERM leaves `thread-writer-locks/<thread>.lock`, a `tmp/arg0/` directory and a
  `shell_snapshots/<thread>…sh` behind; the next launch removed the first two. No
  app-wide write was attributable to this recipe.
- **Native store to capture:**
  `$CODEX_HOME/sessions/**/rollout-*-<thread_id>.jsonl`. Take `<thread_id>` from the first
  stdout event `thread.started`, or from the first new rollout whose `session_meta.cwd`
  equals `<ws>`. The store is shared, so partition by that file.
- **Goal check:** apply the Codex rule of §3.3.
- **Stop criteria:** the process exits after the agent ends its one turn (exit 0 seen).
  Otherwise use the stop procedure above.
- **Refuse before spawn:** an empty or whitespace-only goal, NUL, more than 65536 bytes,
  invalid UTF-8, or an exec size above `ARG_MAX` (§5).

### Claude Code 2.1.283

- **Executable:** exec `~/.local/share/claude/versions/2.1.283` directly, and refuse
  if its sha256 is not
  `d8cb1e5c79684cc12a8bfc813e3a2073406921b6245744b3009be3ab5651d21e`. The
  `~/.local/bin/claude` symlink is moved by other Claude installs' updaters (2.1.281 → 2.1.283
  in three days). All T02 runs executed the symlink, so running the versioned path directly
  (with a different `argv[0]`) is a small unmeasured change.
- **argv template:**
  `[<claude>, "-p", "--model", "haiku", "--effort", "low", "--permission-mode", "acceptEdits", "--permission-prompts", "none", "--session-id", <uuid4 minted before spawn>, "--output-format", "json", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}", "--", <GOAL>]`
- **cwd:** `<ws>`. **stdin:** `/dev/null`. stdout is one JSON result object at exit.
- **Environment:** build it from nothing. Measured:
  - `HOME=<per-run dir>`. Before spawn, create in it exactly one entry: the symlink
    `Library/Keychains` → the user's `~/Library/Keychains`. Subscription login needs it, and
    this keeps `~/.zshrc`, `~/Library/Caches` and `~/.local/state` out of the run (§7.1).
  - `USER` (required for auth), `LOGNAME`, `SHELL=/bin/zsh`, `TMPDIR`,
    `PATH=/usr/bin:/bin:/usr/sbin:/sbin`.
  - `CLAUDE_CONFIG_DIR=<main checkout>/harnesses/home/claude`, `DISABLE_AUTOUPDATER=1`.
  - `ENABLE_CLAUDEAI_MCP_SERVERS=false` and `CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS=1`.
  - **Add for capture:** `HTTPS_PROXY`, `https_proxy`, `NO_PROXY`, `no_proxy` as for Codex,
    and `NODE_EXTRA_CA_CERTS=<proxy CA pem>`. The CA variable is qualified on 2.1.281 only.
  - Never pass `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_URL`, `ANTHROPIC_AUTH_TOKEN`,
    `CLAUDECODE`, other `CLAUDE_CODE_*` or `OTEL_*` from the launching environment.
- **This is a maintainer decision, not just a recipe.** The keychain-link `HOME` is what
  made the Claude recipe app-wide clean. It hides the user's dotfiles (git identity, ssh)
  from the agent, and it writes no version lock (§7.1). The alternative keeps the real
  `HOME`. Then the Bash tool will source and snapshot the participant's `~/.zshrc` (or
  `~/.bashrc` / `~/.profile`), and version locks land in `~/.local/state/claude/locks`.
  That is an app-wide read and a transient app-wide write that no process-scoped setting
  measured here prevents. It is a scoping limitation, not a recipe detail.
  Decided 2026-09-28: this is the default (D38).
- **Workspace:** a fresh empty directory outside any git repository, with no ancestor
  `CLAUDE.md`. `-p` skips the workspace-trust dialog (per `--help`), and no trust entry was
  needed. Do not use `--no-session-persistence`.
- **Known writes (disclose):** into the shared `CLAUDE_CONFIG_DIR`:
  - `.claude.json`, sometimes with a rotated `backups/.claude.json.backup.*`;
  - `projects/<slug>/<session>.jsonl` and `…/memory`;
  - `sessions/`, plus `session-env/<session>` when Bash is used;
  - `plugins/synced/…` and `skills/synced/…` (the account sync);
  - `shell-snapshots/`, only while Bash is in use.

  No app-wide write was attributable to this recipe.
- **Native store to capture:** `$CLAUDE_CONFIG_DIR/projects/*/<session-id>.jsonl`. Glob by
  the pre-assigned id; do not compute the slug. Also watch `<session-id>/subagents/` and
  `<session-id>/tool-results/` beside it (from the V1 manifest; not exercised here).
- **Goal check:** apply the Claude rule of §3.3.
- **Stop criteria:** the process exits after `end_turn` (exit 0 seen). In the one measured
  case, a write needing approval was denied automatically and listed in stdout
  `permission_denials` (§4). Otherwise use the stop procedure above; on transport failure
  Claude retries up to 10 times with backoff.
- **Refuse before spawn:** as for Codex. Claude also refuses an empty or missing goal itself.

## 9. Limits and open items

- The wire leg was not observed (§6), and the Claude CA variable on 2.1.283 is unqualified.
  Both wait on the network-filter allow rule (§8, precondition 1).
- TTY stdin and the interactive TUI launch (goal as the TUI's first prompt) were not
  measured. The recipes are headless (`exec` / `-p`): one turn, no input channel. That is
  compatible with D37, since the tracker supplies no input. The V1 connector manifests ban
  `exec` (Codex) and `-p`/`--print` (Claude) for V1 human-witness reasons. Whether M2
  profiles lift those bans is T14's decision.
- The measured continuation settings are for this measurement only; they do not propose a
  policy for week-long runs. They are Codex `workspace-write` without network, and Claude
  `acceptEdits` with prompts denied. The Bash probes added `--allowedTools Bash` for
  measurement only.
- Claude still lists 31 skills, from the account sync plus built-ins. Codex's developer
  message still describes account apps and plugins. Whether connector tools are reachable in
  a Codex run, and whether `CLAUDE_CODE_SYNC_PLUGINS*` removes Claude's synced skills, were
  not measured.
- The keychain-link `HOME` trade-offs are listed in §7.1 and §8. Whether a Claude updater
  elsewhere could prune 2.1.283 while an unlocked run uses it was not measured.
- Both isolated homes are shared stores. A private per-run `CODEX_HOME` would need a copy of
  `auth.json`, and a token refresh inside the copy would invalidate the shared home's refresh
  token (the 2026-09-24 `refresh_token_reused` failure). That was not attempted. The 6 trust
  entries written in the first round remain in `harnesses/home/codex/config.toml`.
- Runs per variant: n = 1. Timings come from a single loaded machine.

## 10. Artifacts

Committed, sanitized. Each file was checked with `spikes/redact.py` (0 redactions) and
reviewed by eye.

- `docs/qualification/m2/launch-recipes/runs/*.json`: 25 per-run records (16 first-round, 9
  fix-round). They hold argv vectors (goal as `<GOAL>`), environment variable names,
  non-secret path values, goal metadata and text, output check, native analysis, stdout
  summary and, for fix-round runs, `side_effects`. `side_effects` holds names only, with
  account/org ids and long hex ids replaced by `<uuid>`/`<hex>`, and counts only for shell
  snapshots. There are no headers, fingerprints, environment dumps or file contents.
- `docs/qualification/m2/launch-recipes/argmax.json`: the §5 boundary search.
- `docs/qualification/m2/launch-recipes/excerpts/`: native records for the two named rows,
  from the current-recipe runs.
- `docs/qualification/m2/launch-recipes/tools/`: `measure.py` (runner), `mkgoal.py` (exact
  goal bytes) and `sanitize.py` (raw to committed). These are throwaway measurement tools.
  `measure.py` takes `--harnesses` (default `$GOALCUP_HARNESSES`), `--claude-bin` and, with
  `--proxy`, a required `--wprobe PATH`. It carries no machine-specific scratch path. The
  committed version is later than the one that produced the first-round runs. The
  `cx-neg-*` records predate the runner's `home`/`timeout_s` fields, which `sanitize.py`
  backfills.

Raw, git-ignored, in this worktree only:

- `spikes/codex-cli/out/m2-t02/<run-id>/` and `spikes/claude-code-cli/out/m2-t02/<run-id>/`:
  `stdout.txt`, `stderr.txt`, `meta.json` (with unscrubbed side-effect paths) and
  `native-*.jsonl` copies of the session files.
- `…/<run-id>/probe/`: the probe's `wire.jsonl` (masked credential fingerprints, never
  values) and `ca.pem`.
- `spikes/codex-cli/out/m2-t02/probe-selftests/`: the §6 probe records, `conntest.c` and
  `nul.rs`.
