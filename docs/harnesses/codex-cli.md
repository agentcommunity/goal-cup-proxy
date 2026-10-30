# Codex CLI harness facts

Connector id: `codex-cli` · Facts for version: `0.153.0` · Written by: Goal Cup team · Last checked: 2026-10-13
Public disclosure: yes. Everything here comes from the public connector and Goal Cup's own measurements on macOS and, for Linux with no login, in Docker (arm64) and on a GitHub-hosted runner (x86_64); no account data, tokens or personal paths are included.
Status: shipped connector

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64, Codex 0.153.0, 2026-09 and 2026-10 runs); "vendor docs" means a quote that Goal Cup's research file took from OpenAI's Codex documentation or source tree, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. Linux was measured only on arm64 under Docker Desktop (Debian 13 container, Linux 7.0.12, 2026-10-13, Codex 0.153.0, no login, so no model reply); the Linux x86_64 binary was run with no login on a GitHub-hosted `ubuntu-24.04` runner (2026-10-13); a logged-in Linux run was not measured on either architecture. Windows was not measured. Cells for what is still unmeasured say "unknown" or "not measured".

17 Oct update: macOS/Linux staging task, capture, seal and delivery practices passed.
Earlier pending rows below describe the
older measurement checkpoint; longer/resume/platform gaps remain open.

## 18 Oct research checkpoint (Codex 0.156.1)

Measured separately from the historical 0.153.0 rows: macOS arm64, native Homebrew cask
0.156.1, Luna/high, one supplied workload prompt, organizer staging test run. Work completed
in 159.176 seconds; no continuation was injected. Two test runs passed and localhost HTTP 200
was observed. External lookup, dependency installation and native delegation were not exercised.

Actual stored staging bytes were retrieved and replayed: local/rebuilt record count, byte count,
head and research NDJSON agreed. Capture remained incomplete with one shutdown drain deadline,
an unclosed producer and `Unresolved(ProcessAuthorityUnavailable)` termination. An orphan
localhost workload server was identified and stopped separately. Upload completion is therefore
not a clean-capture or process-containment claim. The failed evidence remains private.

The main native turn used high effort. The wire contained 26 high-effort model requests and one
Luna/low automatic thread-title request. Installed-tag source forces low for that auxiliary
request ([thread_title.rs](https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/tui/src/app/thread_title.rs)).
The 19 Oct repeat assigned `/rename Research validation` at the ready prompt before submitting
the workload; the inline name path sends only a name update, and automatic title generation
requires an unnamed thread. All 22 captured repeat requests were Luna/high, with no low-effort
title request observed. This is a harness prelaunch setting, not Goal Cup Proxy or connector policy.

Native delegation was disabled for this experiment: installed-tag full-history forks do not
provide a global guarantee of the requested child model/effort. Native-child coverage remains
unmeasured. Participant settings live only in the existing dedicated capture home. The connector
and shared Goal Cup Proxy code contain no model-specific enforcement or title-generation workaround.

The baseline yielded 564 decoded observations, including 225 unknown records across native and
Responses decoders. Those are partial interpretations of retained source bytes; paired tool
observations on both legs are not separate tool executions.

The fresh-build repeat on 19 Oct (00:24 WITA; 18 Oct UTC), after independent code review and the
fast gate, used the same supplied prompt once and finished in 175.376 seconds. Early manual seal
returned in under one second; no continuation input was supplied. Both local and actual retrieved
staging traces verified valid: 53,129 records, 47,425,207 bytes, zero gaps, identical head, and
byte-identical research NDJSON (439,223 bytes). Signed delivery reached the terminal. The supplied
staging receipt key is not production pinned; identical trust options were used for both replays.
Independent review sampled three passing tests, localhost HTTP 200 and a later HTTP 400 check,
a docs.python.org lookup, and the final assistant reply decoded on the Responses leg. No package
install or browser automation was established; requirements had no non-comment packages.

Termination remained `Unresolved(ProcessAuthorityUnavailable)`. The post-seal check found no
remaining Python/Node/Codex processes whose cwd was under the workload workspace, but cannot
establish containment of every escaped descendant. Native decoder 0.153.0 produced 192 observations
(107 unknown); Responses decoder v2 produced 290 (87 unknown). Both decoder incomplete flags were
false, while interpretation still retains those 194 unknown observations. Context-visible tool
calls/results are available; independent full tool-output collection is outside this experiment.
Native-child, endurance and other-platform coverage remain unmeasured by this repeat.

## 19 Oct focused native-child recording probe

Measured on the same macOS/native Codex 0.156.1 setup: a 23.800-second parent turn spawned one
14.140-second full-context native child. Both actual turns were `gpt-5.6-luna` / `high`.
Inherited model/effort was used; the exposed override list did not offer the legacy Luna slug.
A prior short probe refused before spawning and is retained as a failed experiment.

The supported local-only capture path reused the existing isolated logged-in home; it made no
server run and required no browser approval. The sealed chain verified valid (`local_only`),
2,549 records / 4,451,190 bytes / zero gaps. A blind independent Luna review recovered a private
inherited context marker exactly and found the child assignment, own model/effort, four tool
calls/results and final failure reply. Child identity and parent/fork links are present in the
child's first native `session_meta`; the following header replays the parent's metadata. No
prompt/oracle/workspace file was supplied to the reviewer.

The child attempted further delegation rather than completing its assigned calculation; that
task failed. Recording coverage of the observed child context/actions/reply passed. The decoded
report emits three `human_message` observations for the parent prompt, inherited copied prompt
and child assignment. These must not be treated as three independent human submissions. Decoded
fork lineage/authorship needs separate work; raw source is sufficient for this measured recovery.
This is one full-context child measurement, not proof of every delegation path or global model
restrictions. Original isolated settings and staging routing were restored after the probe.

## 1. Run one prompt, in chat mode first

Chat mode is the product (D57). The connector opens Codex's chat with `codex` and no goal (the args are only the `-c` and `-C` options in section 10); the participant types the prompt; one Ctrl-C at an idle prompt quits and prints the resume line; `resume --last` reopens the most recent session. Section 10 has the argv and the measurements.

Headless recipe (`codex exec`). The connector does not use it; its measurements stand. The headless argv, with the goal as the single element after `--`:

```
codex exec --json -c approval_policy="never" -c check_for_update_on_startup=false \
  -c projects={ "<workspace>" = { trust_level = "trusted" } } \
  -s danger-full-access --skip-git-repo-check -C <workspace> -- "<goal>"
```

- stdin is `/dev/null`; stdout is JSONL events (`thread.started`, `turn.started`, `item.*`, `turn.completed`), stderr is progress text (measured, launch recipes 2026-09-27).
- One turn, then the process exits; exit 0 was seen on every successful goal run (measured). A non-zero exit is expected only on failure (an interactive update prompt that ran `npm install -g` ended a run with `exit status: 1`, measured in a TUI run, not `exec`).
- `-c projects=...` is a per-process override that stops Codex writing a `[projects."<ws>"] trust_level` entry into `$CODEX_HOME/config.toml` (measured: file hash unchanged). The workspace path must contain no `"` or `\`.
- `--skip-git-repo-check` is required when the workspace is not a git repository (measured).
- `exec` accepts an empty goal and runs a model session (measured); Goal Cup refuses empty goals before spawn.
- `codex exec` alone is read-only. `-s danger-full-access` gives tool commands full access and network. That form is not measured; the measured runs used `-m gpt-5.6-luna`, low effort and `workspace-write` without network.
- A 64 KiB goal as one argv element was safe on macOS with an allowlisted environment (measured, `ARG_MAX` 1048576). Other OS limits: unknown.

## 2. Install and record a version

- Distribution: npm package `@openai/codex` (code: `harnesses/install.sh`); a Homebrew cask also exists, and one older cask copy (0.143.0) was on a default `PATH` during a 2026-09-15 run (measured).
- Exact version: install the npm package at the wanted version with the normal npm `@<version>` syntax (not run for this sheet). The repository's install script installs the latest tag.
- Binary: `codex` on `PATH` is an npm wrapper (`node`). The native binary lives in the platform package, `@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex` on macOS arm64 (measured). The connector pins no build (D53): the daemon records the digests of the file `codex` resolves to and of the running image, and does not hash the native binary the wrapper spawns. The connector asks `codex --version` (code: `version_args` in `[capture.executable]`).
- Version: `codex --version` prints `codex-cli 0.153.0` (measured).
- Linux package names and paths (measured: tarballs fetched with `npm pack` and hashed, 2026-10-13): `@openai/codex@0.153.0-linux-x64` holds `package/vendor/x86_64-unknown-linux-musl/bin/codex`, and `@openai/codex@0.153.0-linux-arm64` holds `package/vendor/aarch64-unknown-linux-musl/bin/codex`. The `codex` binary in each Linux package is the musl build (`vendor/<arch>-unknown-linux-musl/bin/codex`, measured). Measured SHA-256 of that file: x86_64 `fce63502...4974be`, arm64 `c92824b2...d40aec`. `codex --version` on Linux prints `codex-cli 0.153.0` and exits 0, with a warning that it won't create PATH helper aliases under a `/tmp` Codex home (measured, Docker `rust:1.99` image on arm64, `--network none`, empty temporary `HOME` and `CODEX_HOME`, 2026-10-13).
- macOS Intel (from the npm tarball, 2026-10-14; not run): `@openai/codex@0.153.0-darwin-x64` holds `package/vendor/x86_64-apple-darwin/bin/codex`.
- Windows package names and paths: unknown. The native binary has different bytes on each OS and architecture, so one SHA-256 names one platform.
- Linux, no login (measured, Linux arm64, Docker Desktop, Debian 13 container, Linux 7.0.12, 2026-10-13, no login): `goalcup doctor` passes the executable check on the arm64 binary, reached through a symlink on `PATH`. Codex tries `wss://api.openai.com/v1/responses` 6 times and gets `401 Unauthorized`, falls back to HTTPS and gets 401 five more times, then emits `turn.failed` and exits 1 about 18 s after exec. The run seals (`AlreadyStopped`) and `goalcup-verify --capture-integrity` exits 0.
- Linux x86_64, no login (measured, GitHub-hosted `ubuntu-24.04` runner, 4 vCPU, ext4, 2026-10-13, [run 38448817041](https://github.com/agentcommunity/goal-cup-proxy/actions/runs/38448817041), `scripts/linux-harness-smoke.sh`): the same path. The x86_64 binary runs, `goalcup doctor` passes, Codex gets `401 Unauthorized` ("Missing bearer or basic authentication") on `wss://` and then on `https://api.openai.com/v1/responses`, emits `turn.failed` and exits 1, all within about 16 s of `goalcup start` beginning. `goalcup start` prints `proxy admission: accepted=18 intercepted=1 auth_refused=2` (the same line on arm64), the run seals `AlreadyStopped`, `goalcup-verify --capture-integrity` exits 0, and no `codex` process is left. That run drove the headless form (`--goal-file`, `codex exec`). Since PR #181 the script reads the connector's manifest instead: a chat-mode connector, as this one is, gets a terminal, one typed prompt, then a hangup, so the outcome above is the headless run's, and this sheet records no chat-mode Linux result.
- Update prompt: Codex 0.153.0 opens an "Update available" selector whose default item runs `npm install -g @openai/codex`. `check_for_update_on_startup = false` stops the prompt and stops Codex writing `version.json` (measured A/B on 0.153.0).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$CODEX_HOME`, default `~/.codex` (vendor docs; measured with an isolated `CODEX_HOME`). Holds `config.toml`, `auth.json`, optional `hooks.json`, `requirements.toml` | unknown (vendor docs name `~/.codex`; not measured on Linux) | unknown |
| Session / transcript store | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<thread_id>.jsonl`; date directory and file name use local time, so glob by thread id (measured). Side files: `history.jsonl`, `session_index.jsonl`, `state_5.sqlite` (measured in a store copy, 2026-09-03) | unknown | unknown |
| Logs / cache | Inside `$CODEX_HOME`: SQLite state, log, memory, queue, goal and thread-history databases; `models_cache.json`; `cache/`; `plugins/cache/`; `shell_snapshots/<thread>.sh`; `thread-writer-locks/`; `tmp/arg0/` (measured, 51 changed paths in one run). `version.json` when the update check is on (measured) | On first start in an empty `CODEX_HOME`, Codex shallow-clones a plugin marketplace with `git` and `git-remote-http` into `CODEX_HOME/.tmp/plugins` (plus `plugins.sha` and `plugins.sync.lock`) and writes `.sandbox_migration` (measured, arm64, no login). The rest: unknown | unknown |
| Env vars that move these | `CODEX_HOME` moves everything above and `auth.json` (measured). `HOME` moves user skills: Codex 0.153.0 reads `$HOME/.agents/skills` and a login shell snapshot of the user's shell startup files (measured). `CODEX_CA_CERTIFICATE` is the CA variable (code) | same variable names are expected; behaviour not measured on Linux | unknown. Windows tools usually read `USERPROFILE` and `APPDATA` rather than `HOME`; whether Codex does is unknown |

Compressed rollouts: vendor docs say a background job compresses rollouts older than 7 days to `.jsonl.zst`. Not observed by Goal Cup.

## 4. Session format and identity

- **File type.** JSONL, one object per line, `{"timestamp", "ordinal"?, "type", "payload"}`. Append-only; the `ordinal` was gap-free per file and stayed contiguous across a kill and a resume (measured, spike 2, 0.153.0).
- **One file per thread.** Normally one rollout file per thread. Vendor docs say a revert keeps the thread id and writes a new immutable file with a `_<rollout_id>` suffix, so watch the directory, not one file. A subagent thread is a separate rollout file; its first `session_meta` carries the parent link, and its second `session_meta` replays the parent's (measured).
- **Id.** In chat mode (section 10) no id is read at launch: the run records every rollout new or written since it started. A UUID-shaped thread id (a time-ordered form, for example `01a0e2ee-...`). It first appears on stdout as `thread.started` with `thread_id` when `--json` is set (measured; the new rollout file appeared 0.12 to 1.17 s after spawn), then as the file-name suffix and as `session_meta.payload.id` / `payload.session_id`. On a later record `payload.id` is an item id, not the thread id, so read `payload.session_id` (measured dogfood finding, 2026-09-07).
- **Human input.** No `event_msg/user_message` record exists on 0.153.0 with paginated history; a typed prompt is a user-role message with the same shape as harness context (measured, spike 2). Do not use the pre-spike claim.
- **Other records.** `turn_context` per turn (model, effort), `token_count` events whose `rate_limits` object carries account plan type, credit balance and quota (measured; Goal Cup redacts it), `response_item`, `compacted`.
- **Prompt history.** `history.jsonl` holds `{session_id, ts, text}` per typed prompt and was the only place a hook-blocked 4,100-character prompt survived (measured).
- **Resume.** `codex resume <id>` appends to the same rollout when it can take `thread-writer-locks/<thread id>.lock`; when it cannot, it silently becomes a new thread with a new id and no back-reference (measured, spike 2).

## 5. Auth

- **Subscription login (used by Goal Cup).** ChatGPT sign-in; `$CODEX_HOME/auth.json` has `auth_mode` `chatgpt` and a refresh time; `codex login status` prints "Logged in using ChatGPT" (measured).
- **API key.** Goal Cup removes `OPENAI_API_KEY` from the child environment (code), so a key set only in your shell does not reach a run. Put an API key in the harness's own settings in its Goal Cup home (`codex-cli/`, which is `$CODEX_HOME`), never on a command line. Goal Cup never reads the key and never sets the address Codex sends requests to. Codex's Responses requests then go to `api.openai.com/v1/responses`, a default host Goal Cup records (`connectors/codex-cli/harness.toml`). Measured with a key: not yet; how Codex stores or uses the key is unknown.
- **Where credentials are stored.** macOS: a file, `$CODEX_HOME/auth.json` (measured; Goal Cup saw it read from `CODEX_HOME`, not a keychain). Linux: unknown. Windows: unknown.
- **Refresh token rotation.** Copying `auth.json` into a second home and refreshing there invalidated the shared home's token (`refresh_token_reused`, measured 2026-09-24). Goal Cup therefore shares one isolated `CODEX_HOME` and serializes runs ([D39](../DECISIONS.md#d39-m2-headless-capture-launches-ported-extra-redaction-patterns-shared-isolated-codex-home)).
- **Files derived from the key.** None known. Account plan and credit data ride in rollout `rate_limits` records (measured).

## 6. Network

- **Model host.** `chatgpt.com`, path `/backend-api/codex/responses`, OpenAI Responses format (code: connector `[[capture.hosts]]`; measured on the 2026-09-28 wire capture). The transport was one WebSocket (`GET` upgrade, 101, permessage-deflate); each client message was a `response.create`. An HTTPS fallback was seen reconnecting in a failed run; a captured HTTP POST plus SSE exchange is not available (measured; Goal Cup fixtures list it as unobserved).
- **Model host with an API key.** `api.openai.com`, path `/v1/responses`, OpenAI Responses format (code: the connector's second `[[capture.hosts]]` row). It is used only when Codex runs on an API key; see section 5. Measured with a key: not yet.
- **Auth host.** `auth.openai.com` (code: `auth_hosts`).
- **Other hosts seen as errors.** A model-list request and MCP / apps requests failed in one run; their hosts: unknown. `codex_apps` MCP server, remote plugin catalog and account plugins are fetched at start (measured as cache files); hosts: unknown.
- **Telemetry.** OTel export is opt-in: `[otel] exporter` defaults to none (vendor docs). Other telemetry hosts: unknown.
- **Update host.** unknown (the prompt runs `npm install -g`, so the npm registry is involved).
- **Proxy.** `HTTPS_PROXY` / `https_proxy` are honoured; a loopback forward proxy relayed the WebSocket (measured, 0.153.0, 2026-09-28). `NO_PROXY` set to loopback by Goal Cup.
- **Custom CA.** `CODEX_CA_CERTIFICATE`, extends the trust roots (connector `capture.ca`, mode `extend`).
- **Base-URL overrides (Goal Cup never sets these).** `OPENAI_BASE_URL`, `CODEX_BASE_URL`; the launcher removes both and preflight refuses them (code).

## 7. Process model

- **Tree.** Three layers (measured): the npm wrapper (`node`), the native `codex` binary, and `codex-code-mode-host` (Codex's own MCP host), which sits in a process group of its own. The `exec` tool runs shell commands as further children.
- **Kill.** Killing the wrapper does not stop the native binary; it survives, reparented, and keeps the thread writer lock (measured, `kill -9`). Goal Cup signals the native binary (`kill.strategy = native_pid`, `native_binary = "codex"`, 10 s grace; code).
- **SIGTERM.** A SIGTERM run left `thread-writer-locks/<thread>.lock`, a `tmp/arg0/` directory and a shell snapshot behind in one measurement; the next launch deleted the first two (measured 2026-09-27). An earlier spike measured that a clean SIGTERM removes the lock. The two measurements disagree; treat residue after SIGTERM as possible.
- **Login shell.** Codex 0.153 runs each agent command as `/bin/sh -lc "<cmd>"`, the shell taken from `/etc/passwd` (measured, rollout of a Docker run, 2026-10-22). A login shell runs `/etc/profile`, which on Debian and Ubuntu sets `PATH` outright, so with an empty `HOME` the participant's `~/.local/bin` (where `install.sh` puts `goalcup`), `~/.cargo/bin` and the rest were lost and `goalcup time-left` failed (measured, Debian container, 2026-10-22). macOS's `/etc/profile` keeps `PATH`. Goal Cup now writes a `.profile` into the run's `HOME` that exports the `PATH` it handed Codex (code: `write_login_profile` in `crates/goalcup-cli/src/home.rs`), so `sh -lc` and `bash -lc` get it back; no Codex setting changes (D67).
- **Subagents.** Separate rollout files and a spawn tool in the `exec` family (measured). Hooks fire inside subagents with an `agent_id` (measured). A subagent's rollout is a `rollout-*.jsonl` file under `$CODEX_HOME/sessions/`, so the connector's one native row (`*/*/*/rollout-*.jsonl` under `sessions`) reads it with the main session; no second row is needed. Its AI calls go to the same model hosts and are recorded on the wire. Not yet checked in a live run with a subagent.
- **Exit codes.** 0 after a completed `exec` turn (measured). 1 on Linux arm64 and x86_64 with no login, after repeated 401 and `turn.failed` (measured). Others: unknown.
- **Orphans.** A shell command the agent started (`sleep`) can outlive the sweep and reparent to init (measured 2026-09-15; see [TERMINATION](../TERMINATION.md) for the current claim). On Linux there was no fork-and-leave on the no-login path: the `git` children Codex forks on first start exited before Codex did, nothing stayed in its process group afterwards, and nothing but Goal Cup Proxy's daemon was reparented to init (measured, Linux arm64, Docker Desktop, Debian 13 container, Linux 7.0.12, 2026-10-13, no login, 1 s sampling; a child that lived and died between samples would not show, so this holds at that sampling interval). A logged-in Linux run, which runs tools, is not measured.

## 8. Things Goal Cup cannot isolate

- **Shared home.** All runs share one isolated `CODEX_HOME` (D39), so runs must be serialized and the directory holds older runs' sessions.
- **Account state on OpenAI's side.** Plugins, apps, connectors and memory summary arrive from the account and appear in the developer message (measured); rate-limit and plan data ride on every turn.
- **No sandbox from Goal Cup.** The connector passes no sandbox, approval or model option (D52, D57). Codex runs with its own defaults and the participant's settings; Goal Cup recommends some ([Run without permission prompts](#run-without-permission-prompts), D67). The per-run `HOME` stops automatic loading of `~/.agents/skills` and shell startup files, not deliberate reads or writes.
- **Machine-wide or managed settings.** The connector seeds no `requirements.toml` in `CODEX_HOME`. A system-wide managed config location: unknown.
- **A second Codex.** Another `CODEX_HOME`, an older global Homebrew copy, or the Codex desktop app can run beside the tracked one; the app shares `CODEX_HOME` (vendor docs; Goal Cup measured an app thread in 2026-09).

## 9. Quirks and open questions

- Modal prompts that swallow a typed turn on the TUI: update selector, "Approaching rate limits - switch model?" and two model-migration prompts. The `[tui]` keys `hide_rate_limit_model_nudge`, `hide_gpt5_1_migration_prompt` and `"hide_gpt-5.1-codex-max_migration_prompt"` exist in the 0.153.0 binary (measured; the last key must be quoted in TOML).
- Untrusted hook definitions are skipped silently, with exit 0 and no warning (measured, spike 2).
- Hooks are discovered from the working directory, not the git root (measured).
- Rollout file and directory names use local time, not UTC (measured).
- Rollout `user` and `developer` messages without the `user.text` marker are the harness's own context (environment context, permissions, skill, plugin and multi-agent instructions, a model switch note; measured on 0.153.0). Decoder `codex-native` v4 reports each as `unknown` `unmarked_message` with its role and supplied `content_item_kinds`, never as human input and with no text (code).
- Open: everything still marked "unknown" above for Linux and Windows (a logged-in run, the login location, the config-dir default); the managed-config path; the API-key path; the hosts for model lists, apps and updates; whether SIGTERM removes the writer lock.
- The Linux facts come from the 2026-10-13 arm64 Docker run and the x86_64 runner smoke.

## 10. Chat mode (measured 14 Oct 2026)

The connector runs Codex in its normal chat mode in the participant's own terminal, and the participant types the prompt (code: [connectors/codex-cli/harness.toml](../../connectors/codex-cli/harness.toml), `interactive = true`). The argv is the args alone, with no goal and no `exec`:

```
codex -c check_for_update_on_startup=false \
  -c projects={ "<workspace>" = { trust_level = "trusted" } } -C <workspace>
```

identity `none`; native root `{config_home}/sessions`, pattern `*/*/*/rollout-*.jsonl`, `since_run_start = true` (every rollout new or written since the run started is recorded); `skills_dir = "skills"` (`goalcup start` puts the goal-cup skill in `$CODEX_HOME/skills/goal-cup/` before each launch); `resume_args = ["resume", "--last"]`, what `goalcup continue` appends (not measured, see PENDING below).

Measured on macOS arm64, Codex 0.153.0, 2026-10-14, by driving the TUI on a pseudo-terminal (120x40) in a fresh `git init` workspace, with `CODEX_HOME` Goal Cup Proxy's isolated home and an empty `HOME`. That home was **logged out** (an expired refresh token), so no model reply was seen:

- No trust prompt appears with the `projects` trust override.
- The rollout file is **not** created at startup (none after 15 s at an idle prompt). It is created when the first message is submitted (its birth time is the Enter), under `sessions/YYYY/MM/DD/`, and its name carries the startup time: `rollout-<startup time>-<thread id>.jsonl`.
- The typed message is in that file twice: a `response_item` (role `user`, `input_text`) and an `event_msg` `item_completed` (`UserMessage`).
- One Ctrl-C at an idle prompt ends Codex, which prints `To continue this session, run: codex resume <id>`.
- `codex <args> resume --last` is accepted by the parser (the global options before the subcommand are fine).
- An MCP server configured in the shared home whose token had expired failed at startup (HTTP 401, "MCP startup incomplete"); Codex carried on.

PENDING (need a logged-in home, which the owner must restore; nobody on our side logs in for a test):

- Whether `codex <args> resume --last` in Goal Cup Proxy's home reopens the same rollout file or starts a new one. The connector ships `resume_args = ["resume", "--last"]` on the plan's word, not on a measurement.
- A model reply in chat mode through the recording proxy.
- One 10-minute practice run of this connector on this Mac against staging.

## Run without permission prompts

During a run you may type only "keep going" ([RULES](../RULES.md)), so a Codex that stops to ask for
an approval waits until the deadline. Goal Cup recommends these two settings and never sets them
for you (D67). They go in `config.toml` in Codex's Goal Cup home (`CODEX_HOME`, under
`capture-homes/codex-cli/` in your Goal Cup configuration folder; `goalcup setup` prints the full path
when one is missing):

```toml
approval_policy = "never"
sandbox_mode = "danger-full-access"
```

- `approval_policy = "never"`: Codex never asks; a command that fails is returned to the model
  (`codex --help`, 0.156.1). Without it, Codex asks whenever its sandbox blocks a command.
- `sandbox_mode = "danger-full-access"`: no sandbox. The default `workspace-write` sandbox makes
  `.git` read-only: `git add` failed with `Operation not permitted` on `.git/index.lock`, and Codex
  then asked and waited 8.7 of 10 minutes (mac-codex, local test day). In a Linux container the
  sandbox cannot start at all (`bwrap: No permissions to create a new namespace`), so every command
  needs an approval (lx-codex). With `never` alone, those commands fail instead and
  the agent cannot commit.
- Without a sandbox the agent's commands can change anything your user can. Run it on a machine
  or in a container you are content to let it change.
- `goalcup setup` reads both keys and, while one is not set this way, adds the recommendation to its
  harness home step. It never changes them and never fails for them. The test kit
  (`scripts/kit_settle.py`) sets both in its own cells.
- Evidence: [local test day](../research/results/2026-10-22-local-test-day.md),
  [plan v4](../research/results/2026-10-22-fix-plan-v4.md).
- Every setting for a long run, for participants:
  [Set up your harness for a 168-hour run](../HARNESS-SETUP.md#codex).
