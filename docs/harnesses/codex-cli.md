# Codex CLI harness facts

Connector id: `codex-cli` · Facts for version: `0.153.0` · Written by: Goal Cup team · Last checked: 2026-10-13
Public disclosure: yes. Everything here comes from the public connector, the public decision log and Goal Cup's own measurements on macOS and, for Linux arm64 with no login, in Docker; no account data, tokens or personal paths are included.
Status: shipped connector

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64, Codex 0.153.0, 2026-09 and 2026-10 runs); "vendor docs" means a quote that Goal Cup's research file took from OpenAI's Codex documentation or source tree, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. Linux was measured only on arm64 under Docker Desktop (Debian 13 container, Linux 7.0.12, 2026-10-13, Codex 0.153.0, no login, so no model reply); the Linux x86_64 binary was hashed but never run, and a logged-in Linux run has not happened. Windows was not measured. Cells for what is still unmeasured say "unknown".

## 1. Run one prompt, headless

The capture argv (code: [connectors/codex-cli/harness.toml](../../connectors/codex-cli/harness.toml)), with the goal appended as the single element after `--`:

```
codex exec --json -m gpt-5.6-luna -c model_reasoning_effort="low" -c approval_policy="never" \
  -c check_for_update_on_startup=false -c projects={ "<workspace>" = { trust_level = "trusted" } } \
  -s workspace-write --skip-git-repo-check -C <workspace> -- "<goal>"
```

- stdin is `/dev/null`; stdout is JSONL events (`thread.started`, `turn.started`, `item.*`, `turn.completed`), stderr is progress text (measured, launch recipes 2026-09-27).
- One turn, then the process exits; exit 0 was seen on every successful goal run (measured). A non-zero exit is expected only on failure (an interactive update prompt that ran `npm install -g` ended a run with `exit status: 1`, measured in a TUI run, not `exec`).
- `-c projects=...` is a per-process override that stops Codex writing a `[projects."<ws>"] trust_level` entry into `$CODEX_HOME/config.toml` (measured: file hash unchanged). The workspace path must contain no `"` or `\`.
- `--skip-git-repo-check` is required when the workspace is not a git repository (measured).
- `exec` accepts an empty goal and runs a model session (measured); Goal Cup refuses empty goals before spawn.
- Tool commands run in `workspace-write` with no network (`network_access: false`, measured). The sandbox profile grants read access to the whole filesystem (measured).
- A 64 KiB goal as one argv element was safe on macOS with an allowlisted environment (measured, `ARG_MAX` 1048576). Other OS limits: unknown.

## 2. Install and pin a version

- Distribution: npm package `@openai/codex` (code: `harnesses/install.sh`); a Homebrew cask also exists, and one older cask copy (0.143.0) was on a default `PATH` during a 2026-09-15 run (measured).
- Exact version: install the npm package at the wanted version with the normal npm `@<version>` syntax (not run for this sheet). The repository's install script installs the latest tag, not a pinned one.
- Binary: `codex` on `PATH` is an npm wrapper (`node`). The native binary lives in the platform package, `@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex` on macOS arm64 (measured). Goal Cup pins the native binary by SHA-256 (code: `[capture.executable]` in the connector), not the wrapper.
- Version: `codex --version` prints `codex-cli 0.153.0` (measured).
- Linux package names and paths (measured: tarballs fetched with `npm pack` and hashed, 2026-10-13): `@openai/codex@0.153.0-linux-x64` holds `package/vendor/x86_64-unknown-linux-musl/bin/codex`, and `@openai/codex@0.153.0-linux-arm64` holds `package/vendor/aarch64-unknown-linux-musl/bin/codex`. Each package contains only the musl build (measured). Goal Cup pins the SHA-256 of that file per architecture (code: `[capture.platform.linux-x86_64]` and `[capture.platform.linux-aarch64]` in the connector): x86_64 `fce63502...4974be`, arm64 `c92824b2...d40aec`. The arm64 hash was re-measured on a Mac with `shasum -a 256` and in the container with `sha256sum`; the x86_64 binary was only hashed, never run.
- Windows package names and paths: unknown. Whether the native binary has the same bytes on another OS or architecture: it does not; one SHA-256 pins one platform and one architecture (design note in goal-cup-tracer history, `docs/research/platform-portability.md`, seam 7).
- Linux, no login (measured, Linux arm64, Docker Desktop, Debian 13 container, Linux 7.0.12, 2026-10-13, no login): `goalcup doctor` passes the executable check on the pinned arm64 binary, reached through a symlink on `PATH` (the pin is checked on the file behind the link). Codex tries `wss://api.openai.com/v1/responses` 6 times and gets `401 Unauthorized`, falls back to HTTPS and gets 401 five more times, then emits `turn.failed` and exits 1 about 18 s after exec. The run seals (`AlreadyStopped`) and `goalcup-verify --capture-integrity` exits 0. `codex --version` on Linux prints `codex-cli 0.153.0` and exits 0, with a warning that it won't create PATH helper aliases under a `/tmp` Codex home (measured, Linux arm64, Docker, `--network none`, empty temporary homes, 2026-10-13).
- Update prompt: Codex 0.153.0 opens an "Update available" selector whose default item runs `npm install -g @openai/codex`. `check_for_update_on_startup = false` stops the prompt and stops Codex writing `version.json` (measured A/B on 0.153.0).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$CODEX_HOME`, default `~/.codex` (vendor docs; measured with an isolated `CODEX_HOME`). Holds `config.toml`, `auth.json`, optional `hooks.json`, `requirements.toml` | unknown (vendor docs name `~/.codex`; not measured on Linux) | unknown |
| Session / transcript store | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<thread_id>.jsonl`; date directory and file name use local time, so glob by thread id (measured). Side files: `history.jsonl`, `session_index.jsonl`, `state_5.sqlite` (measured in a store copy, 2026-09-03) | unknown | unknown |
| Logs / cache | Inside `$CODEX_HOME`: SQLite state, log, memory, queue, goal and thread-history databases; `models_cache.json`; `cache/`; `plugins/cache/`; `shell_snapshots/<thread>.sh`; `thread-writer-locks/`; `tmp/arg0/` (measured, 51 changed paths in one run). `version.json` when the update check is on (measured) | On first start in an empty `CODEX_HOME`, Codex shallow-clones a plugin marketplace with `git` and `git-remote-http` into `CODEX_HOME/.tmp/plugins` (plus `plugins.sha` and `plugins.sync.lock`) and writes `.sandbox_migration` (measured, arm64, no login). The rest: unknown | unknown |
| Env vars that move these | `CODEX_HOME` moves everything above and `auth.json` (measured). `HOME` moves user skills: Codex 0.153.0 reads `$HOME/.agents/skills` and a login shell snapshot of the user's shell startup files (measured). `CODEX_CA_CERTIFICATE` is the CA variable (code) | same variable names are expected; behaviour not measured on Linux | unknown. Goal Cup's own design note says Windows tools read `USERPROFILE`/`APPDATA` rather than `HOME`; whether Codex does is unknown |

Compressed rollouts: vendor docs (research file, from Codex source) say a background job compresses rollouts older than 7 days to `.jsonl.zst`. Not observed by Goal Cup.

## 4. Session format and identity

- **File type.** JSONL, one object per line, `{"timestamp", "ordinal"?, "type", "payload"}`. Append-only; the `ordinal` was gap-free per file and stayed contiguous across a kill and a resume (measured, spike 2, 0.153.0).
- **One file per thread.** Normally one rollout file per thread. Vendor docs say a revert keeps the thread id and writes a new immutable file with a `_<rollout_id>` suffix, so watch the directory, not one file. A subagent thread is a separate rollout file; its first `session_meta` carries the parent link, and its second `session_meta` replays the parent's (measured).
- **Id.** A UUID-shaped thread id (a time-ordered form, for example `01a0e2ee-...`). It first appears on stdout as `thread.started` with `thread_id` when `--json` is set (measured; the new rollout file appeared 0.12 to 1.17 s after spawn), then as the file-name suffix and as `session_meta.payload.id` / `payload.session_id`. On a later record `payload.id` is an item id, not the thread id, so read `payload.session_id` (measured dogfood finding, 2026-09-07).
- **Human input.** No `event_msg/user_message` record exists on 0.153.0 with paginated history; a typed prompt is a user-role message with the same shape as harness context (measured, spike 2). Do not use the pre-spike claim.
- **Other records.** `turn_context` per turn (model, effort), `token_count` events whose `rate_limits` object carries account plan type, credit balance and quota (measured; Goal Cup redacts it), `response_item`, `compacted`.
- **Prompt history.** `history.jsonl` holds `{session_id, ts, text}` per typed prompt and was the only place a hook-blocked 4,100-character prompt survived (measured).
- **Resume.** `codex resume <id>` appends to the same rollout when it can take `thread-writer-locks/<thread id>.lock`; when it cannot, it silently becomes a new thread with a new id and no back-reference (measured, spike 2).

## 5. Auth

- **Subscription login (used by Goal Cup).** ChatGPT sign-in; `$CODEX_HOME/auth.json` has `auth_mode` `chatgpt` and a refresh time; `codex login status` prints "Logged in using ChatGPT" (measured).
- **API key.** Goal Cup removes `OPENAI_API_KEY` from the child environment (code), so a key path exists; how it is stored or used: unknown.
- **Where credentials are stored.** macOS: a file, `$CODEX_HOME/auth.json` (measured; Goal Cup saw it read from `CODEX_HOME`, not a keychain). Linux: unknown. Windows: unknown.
- **Refresh token rotation.** Copying `auth.json` into a second home and refreshing there invalidated the shared home's token (`refresh_token_reused`, measured 2026-09-24). Goal Cup therefore shares one isolated `CODEX_HOME` and serializes runs ([D39](../DECISIONS.md#d39-m2-headless-capture-launches-ported-extra-redaction-patterns-shared-isolated-codex-home)).
- **Files derived from the key.** None known. Account plan and credit data ride in rollout `rate_limits` records (measured).

## 6. Network

- **Model host.** `chatgpt.com`, path `/backend-api/codex/responses`, OpenAI Responses format (code: connector `[[capture.hosts]]`; measured on the 2026-09-28 wire capture). The transport was one WebSocket (`GET` upgrade, 101, permessage-deflate); each client message was a `response.create`. An HTTPS fallback was seen reconnecting in a failed run; a captured HTTP POST plus SSE exchange is not available (measured; Goal Cup fixtures list it as unobserved).
- **Auth host.** `auth.openai.com` (code: `auth_hosts`).
- **Other hosts seen as errors.** A model-list request and MCP / apps requests failed in one run; their hosts: unknown. `codex_apps` MCP server, remote plugin catalog and account plugins are fetched at start (measured as cache files); hosts: unknown.
- **Telemetry.** OTel export is opt-in: `[otel] exporter` defaults to none (vendor docs). Other telemetry hosts: unknown.
- **Update host.** unknown (the prompt runs `npm install -g`, so the npm registry is involved).
- **Proxy.** `HTTPS_PROXY` / `https_proxy` are honoured; a loopback forward proxy relayed the WebSocket (measured, 0.153.0, 2026-09-28). `NO_PROXY` set to loopback by Goal Cup.
- **Custom CA.** `CODEX_CA_CERTIFICATE`, extends the trust roots (code: CA registry, mode `extend`).
- **Base-URL overrides (Goal Cup never sets these).** `OPENAI_BASE_URL`, `CODEX_BASE_URL`; the launcher removes both and preflight refuses them (code).

## 7. Process model

- **Tree.** Three layers (measured): the npm wrapper (`node`), the native `codex` binary, and `codex-code-mode-host` (Codex's own MCP host), which sits in a process group of its own. The `exec` tool runs shell commands as further children.
- **Kill.** Killing the wrapper does not stop the native binary; it survives, reparented, and keeps the thread writer lock (measured, `kill -9`). Goal Cup signals the native binary (`kill.strategy = native_pid`, `native_binary = "codex"`, 10 s grace; code).
- **SIGTERM.** A SIGTERM run left `thread-writer-locks/<thread>.lock`, a `tmp/arg0/` directory and a shell snapshot behind in one measurement; the next launch deleted the first two (measured 2026-09-27). An earlier spike measured that a clean SIGTERM removes the lock. The two measurements disagree; treat residue after SIGTERM as possible.
- **Subagents.** Separate rollout files and a spawn tool in the `exec` family (measured). Hooks fire inside subagents with an `agent_id` (measured).
- **Exit codes.** 0 after a completed `exec` turn (measured). 1 on Linux arm64 with no login, after repeated 401 and `turn.failed` (measured). Others: unknown.
- **Orphans.** A shell command the agent started (`sleep`) can outlive the sweep and reparent to init (measured 2026-09-15; see [TERMINATION](../TERMINATION.md) for the current claim). On Linux there was no fork-and-leave on the no-login path: the `git` children Codex forks on first start exited before Codex did, nothing stayed in its process group afterwards, and nothing but the tracker's daemon was reparented to init (measured, Linux arm64, Docker Desktop, Debian 13 container, Linux 7.0.12, 2026-10-13, no login, 1 s sampling). A logged-in Linux run, which runs tools, is not measured.

## 8. Things Goal Cup cannot isolate

- **Shared home.** All runs share one isolated `CODEX_HOME` (D39), so runs must be serialized and the directory holds older runs' sessions.
- **Account state on OpenAI's side.** Plugins, apps, connectors and memory summary arrive from the account and appear in the developer message (measured); rate-limit and plan data ride on every turn.
- **Read-everywhere sandbox.** The `workspace-write` profile still grants filesystem read access (measured); the per-run `HOME` stops automatic loading of `~/.agents/skills` and shell startup files, not deliberate reads.
- **Machine-wide or managed settings.** `requirements.toml` inside `CODEX_HOME` was on the old V1 connector's allowlist (git history before `e4993a2`); the current connector does not seed it. A system-wide managed config location: unknown.
- **A second Codex.** Another `CODEX_HOME`, an older global Homebrew copy, or the Codex desktop app can run beside the tracked one; the app shares `CODEX_HOME` (vendor docs via research file; Goal Cup measured an app thread in 2026-09).

## 9. Quirks and open questions

- Modal prompts that swallow a typed turn on the TUI: update selector, "Approaching rate limits - switch model?" and two model-migration prompts. The `[tui]` keys `hide_rate_limit_model_nudge`, `hide_gpt5_1_migration_prompt` and `"hide_gpt-5.1-codex-max_migration_prompt"` exist in the 0.153.0 binary (measured; the last key must be quoted in TOML).
- Untrusted hook definitions are skipped silently, with exit 0 and no warning (measured, spike 2).
- Hooks are discovered from the working directory, not the git root (measured).
- Rollout file and directory names use local time, not UTC (measured).
- Open: everything still marked "unknown" above for Linux and Windows (a logged-in run, the login location, the config-dir default, the x86_64 binary's behaviour); the managed-config path; the API-key path; the hosts for model lists, apps and updates; whether SIGTERM removes the writer lock.
- Compiled from records dated 2026-09-03 to 2026-10-07 and the public connector; none of the runs were repeated for this sheet. The Linux facts come from the 2026-10-13 arm64 Docker run (PR #55).
