# Claude Code CLI harness facts

Connector id: `claude-code-cli` · Facts for version: `2.1.283` · Written by: Goal Cup team · Last checked: 2026-10-13
Public disclosure: yes. Everything here comes from the public connector, the public decision log and Goal Cup's own measurements on macOS and, for Linux with no login, in Docker (arm64) and on a GitHub-hosted runner (x86_64); no account data, tokens or personal paths are included.
Status: shipped connector

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64; Claude Code 2.1.283 unless another version is named); "vendor docs" means a quote that Goal Cup's research file took from Anthropic's Claude Code documentation, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. Linux was measured only on arm64 under Docker Desktop (Debian 13 container, Linux 7.0.12, 2026-10-13, Claude Code 2.1.283, no login, so no model call); the Linux x86_64 binary was run with no login on a GitHub-hosted `ubuntu-24.04` runner (2026-10-13); a logged-in Linux run was not measured on either architecture. Windows was not measured. Cells for what is still unmeasured say "unknown" or "not measured".

## 1. Run one prompt, headless

The capture argv (code: [connectors/claude-code-cli/harness.toml](../../connectors/claude-code-cli/harness.toml)), goal appended as the single element after `--`:

```
claude -p --permission-mode bypassPermissions \
  --session-id <uuid minted before spawn> --output-format json -- "<goal>"
```

- stdin is `/dev/null`; stdout is one JSON result object at exit (measured). Exit 0 after `end_turn` (measured).
- A missing or empty goal makes Claude exit 1 with `Input must be provided either through stdin or as a prompt argument when using --print` (measured).
- A write that needs approval is denied automatically and listed in the result's `permission_denials` (measured).
- `--session-id` must be a valid UUID and pins the id, the transcript file name, record contents and hook payloads (measured; vendor docs).
- Do not pass `--no-session-persistence` (it suppresses the transcript). `--bare` skips hooks and CLAUDE.md but does not use the subscription login (vendor docs).
- On transport failure Claude retries with backoff, up to 10 times by default on 2.1.283 (measured: retries 1 to 7 of 10 seen, backoff 0.6 s to 38 s). The connector sets `CLAUDE_CODE_RETRY_WATCHDOG=1`, which vendor docs say raises this for unattended sessions (code comment; not measured by Goal Cup).
- A 64 KiB goal as one argv element was safe on macOS (measured). Other OS limits: unknown.

## 2. Install and pin a version

- Installed form on macOS: `~/.local/bin/claude` is a symlink to `~/.local/share/claude/versions/2.1.283`, a single native executable (measured). Other installs' updaters move the symlink (2.1.281 to 2.1.283 in three days, measured), so Goal Cup execs the versioned file and records its digest; no build is pinned (D53).
- Print the version: `claude --version`. The connector's `version_args = ["--version"]` asks it once per run and records the line as `Launch.harness_version` (D53); no version is pinned.
- Install an exact version: unknown (not recorded by Goal Cup).
- Stop self-update: `DISABLE_AUTOUPDATER=1` (code: connector env).
- Version lock: while a run is going Claude creates and removes `~/.local/state/claude/locks/2.1.283.lock`; `XDG_STATE_HOME` redirects it (measured).
- Linux packages (measured: tarballs fetched with `npm pack` and hashed, 2026-10-13): `@anthropic-ai/claude-code-linux-x64@2.1.283` and `@anthropic-ai/claude-code-linux-arm64@2.1.283`, each holding the native executable at `package/claude`. The packages without a `-musl` suffix are the ones measured (assumed glibc builds; linkage not checked); the `-musl` packages were not measured. Measured SHA-256 of that file per architecture (the connector pinned these until D53; it now pins no build): x86_64 `1859583c...804ae2`, arm64 `346d294f...15e697`. The arm64 hash was re-measured on a Mac with `shasum -a 256` and in the container with `sha256sum`; the x86_64 hash was re-checked with `sha256sum` on an x86_64 runner before it ran. `claude --version` on Linux prints `2.1.283 (Claude Code)` and exits 0 (measured, Docker `rust:1.99` image on arm64, `--network none`, empty temporary `HOME` and `CLAUDE_CONFIG_DIR`, 2026-10-13). Linux install locations of the native installer: unknown (not tried).
- Linux, no login (measured, Linux arm64, Docker Desktop, Debian 13 container, Linux 7.0.12, 2026-10-13, no login): `goalcup doctor` passes the executable check on the then-pinned arm64 binary, reached through a symlink on `PATH` (the file behind the link is what is hashed). Claude returns `{"is_error":true,"result":"Not logged in · Please run /login","terminal_reason":"api_error"}` with `duration_api_ms` 0 and `total_cost_usd` 0 in about 49 ms, and exits 1; the proxy saw one intercepted connection, closed `interrupted`, with no wire stream. The run seals (`AlreadyStopped`) and `goalcup-verify --capture-integrity` exits 0.
- Linux x86_64, no login (measured, GitHub-hosted `ubuntu-24.04` runner, 4 vCPU, ext4, 2026-10-13, [run 38448817041](https://github.com/agentcommunity/goal-cup-tracer/actions/runs/38448817041), `scripts/linux-harness-smoke.sh`): `claude --version` prints `2.1.283 (Claude Code)`, `goalcup doctor` passes, Claude returns `{"is_error":true,"result":"Not logged in · Please run /login"}` with `duration_api_ms` 0 and `total_cost_usd` 0 in 126 ms and exits 1, `goalcup start` prints `proxy admission: accepted=5 intercepted=5`, the run seals `AlreadyStopped`, `goalcup-verify --capture-integrity` exits 0, and no `claude` process is left.
- macOS Intel (hashed from the npm tarball, 2026-10-14; not run): `@anthropic-ai/claude-code-darwin-x64@2.1.283` holds `package/claude`. The same method on `-darwin-arm64` reproduces the former base pin.
- Windows install locations: unknown. A SHA-256 matches one platform and one architecture only, which is why builds are recorded rather than pinned (D53) (design note in goal-cup-tracer history, `docs/research/platform-portability.md`, seam 7).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$CLAUDE_CONFIG_DIR`, default `~/.claude` (vendor docs; measured with an isolated dir). Holds `settings.json`, `settings.local.json`, `.claude.json` (account and onboarding state; default location without the variable is `~/.claude.json`), `plugins/synced/`, `skills/synced/`, `sessions/`, `session-env/`, `shell-snapshots/` (measured) | unknown (vendor docs name `~/.claude`; not measured on Linux) | unknown |
| Session / transcript store | `$CLAUDE_CONFIG_DIR/projects/<slug>/<session-id>.jsonl`, slug = working directory with every non-alphanumeric character mapped to `-` (measured; vendor docs: truncated to 200 characters plus a hash when longer, not exercised). Subagents: `<session-id>/subagents/agent-<agentId>.jsonl` and `.meta.json`. Large tool output spills to `<session-id>/tool-results/<id>.txt` (vendor docs; the spill path was not exercised by Goal Cup) | unknown | unknown |
| Logs / cache | `~/Library/Caches/claude-cli-nodejs/<slug>/mcp-logs-<server>/` when MCP servers start (measured; not written with MCP off); `~/.local/state/claude/locks/` (measured); `shell-snapshots/` in the config dir, removed at clean exit (measured) | unknown | unknown |
| Env vars that move these | `CLAUDE_CONFIG_DIR` (config, transcripts); `HOME` (the Bash tool sources the user's `~/.zshrc` via the home directory, not `ZDOTDIR`; cache path); `XDG_STATE_HOME` (version lock only); `XDG_CACHE_HOME` reaches only the updater staging dir, not the MCP cache; `CLAUDE_CODE_PROJECT_DIR_NAME` (vendor docs, 2.1.234 and later); `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` stops history writing (vendor docs) | same variable names are expected; behaviour not measured on Linux | unknown |

Retention: vendor docs say transcripts are deleted after `cleanupPeriodDays` (default 30).

## 4. Session format and identity

- **File type.** JSONL, one file per session, appended as the session runs. Vendor docs and Goal Cup's measurements agree it is mostly append-only but not guaranteed: writes are asynchronous, timestamps are not monotonic in file order (27 of 968 records in one file), and docs describe replaced files named `<session>.jsonl.superseded-<timestamp>` and `<session>.orphaned-...jsonl`. Track by path and inode.
- **Id.** A UUID. With `--session-id` it is fixed before spawn, so it first appears in the argv, then in the file name (the file appeared 0.35 to 2.4 s after spawn), in every record's `sessionId`, and in the stdout result's `session_id` (measured).
- **Records.** Each record carries `uuid`, `parentUuid`, `isSidechain`, `sessionId`, `timestamp`, `cwd`, `gitBranch`, `version`, `entrypoint`. Types include `user`, `assistant`, `attachment`, `system`, `queue-operation` (measured on 2.1.181 to 2.1.258). Anthropic states the format is internal and changes between versions (vendor docs).
- **Human input signals.** `origin.kind` (`human`, `channel`, `peer`, `task-notification`, ...), `promptSource` (`typed`, `sdk`, `system`), `isMeta`, `isCompactSummary` (measured and vendor docs). A subagent-completion notice arrived through the prompt hook with no meta flag (measured, 2.1.259).
- **Model.** `message.model` on every assistant record (measured).
- **Resume.** `--resume <id>` appends to the same file; resuming in two terminals interleaves them (vendor docs). `--fork-session` copies to a new id. Resume after a kill needs 2.1.259 or later (code: tested range).
- **Not complete.** The transcript omits the system-reminder block, tool schemas and the billing header that the API request carries (measured by diffing against raw bodies, 2.1.258).

## 5. Auth

- **Subscription login (used by Goal Cup).** claude.ai OAuth. `claude auth status` reported `loggedIn true`, `authMethod claude.ai`, subscription `max` (measured).
- **API key paths.** `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`. Goal Cup never passes them (code). Amp removes these from Claude's environment in its Claude Code mode (Amp docs, via Goal Cup's Amp record).
- **Where credentials are stored.** macOS: the login Keychain, shared across config directories (code comment; consistent with a measurement: `loggedIn` was false with an empty `HOME` and true again once `HOME` held only a `Library/Keychains` link to the real keychain directory). Account metadata (`oauthAccount`, `userID`) sits in `.claude.json` (code: connector seeds these keys). Linux: unknown (measured on arm64 with no login: the no-login error names no path, and the run `HOME`'s dangling `Library/Keychains` symlink, from the connector's `keychain_link` home, drew no complaint); Goal Cup's design note believes a credentials file under the config dir, not measured. Windows: unknown.
- **Auth needs `USER`.** `loggedIn` was false when `USER` was absent from the environment; setting only `LOGNAME` did not help (measured).
- **Files derived from the key.** None known.

## 6. Network

- **Model host.** `api.anthropic.com`, `POST /v1/messages`, HTTP/1.1, 200 `text/event-stream` (measured, 2.1.283, 2026-09-28 wire capture). Request 2 was a threaded continuation.
- **Auth host.** `platform.claude.com` (code: `auth_hosts`).
- **Connectors and sync.** At each launch Claude starts the account's claude.ai connectors and account-synced plugin MCP servers unless `ENABLE_CLAUDEAI_MCP_SERVERS=false`, `CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS=1` and `--strict-mcp-config` are set (measured; which one suffices alone was not isolated). The capture launch sets none of them since 2026-10-14 (owner direction: no MCP limit), so their traffic is tunnelled and not recorded. Earlier measured runs used all three, with `--model haiku --effort low --permission-mode acceptEdits`; the default-model, bypassed-permission launch is not yet measured. Hosts: unknown. Plugins and skills are refreshed from the account at each launch (measured).
- **Telemetry.** OpenTelemetry export is opt-in via `CLAUDE_CODE_ENABLE_TELEMETRY=1` and `OTEL_*` (vendor docs). Any default telemetry hosts: unknown.
- **Update host.** unknown (the updater is disabled with `DISABLE_AUTOUPDATER=1`).
- **Proxy.** `HTTPS_PROXY` is honoured; a loopback forward proxy captured the exchange above (measured, 2.1.283). Transport is plain HTTP/1.1 with server-sent events; no WebSocket seen.
- **Custom CA.** `NODE_EXTRA_CA_CERTS`, extends the roots (code: CA registry). The live check was qualified on 2.1.281 first; the 2.1.283 wire capture then worked with it.
- **Base-URL overrides (Goal Cup never sets these).** `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_URL`; vendor docs say `ANTHROPIC_BASE_URL` works with a subscription login and disables Remote Control.

## 7. Process model

- **Tree.** One native executable. The Bash tool starts `zsh -c -l <script>`; MCP servers are children; subagents run in-process and write sidechain transcript files (measured).
- **Kill.** Goal Cup signals the process group, 10 s grace, then SIGKILL (code: `strategy = process_group`). SIGTERM-specific behaviour of Claude: unknown.
- **Hard kill.** `kill -9` leaves no session-end record; request body files written through the OTel raw-body switch were complete JSON up to the kill (measured, 2.1.259).
- **Exit codes.** 0 after a finished turn, 1 for a missing goal (measured). 1 on Linux arm64 and x86_64 with no login (measured). Others: unknown.
- **Orphans.** On Linux there was no fork-and-leave on the no-login path: Claude's three short-lived children exited before it did, nothing stayed in its process group afterwards, and nothing but the tracker's daemon was reparented to init (measured, Linux arm64, Docker Desktop, Debian 13 container, Linux 7.0.12, 2026-10-13, no login, 0.1 s sampling; a child that lived and died between samples would not show, so this holds at that sampling interval). A logged-in Linux run, which runs tools, is not measured.

## 8. Things Goal Cup cannot isolate

- **Keychain.** The login lives in the user's real Keychain; the run `HOME` links `Library/Keychains` to it ([D38](../DECISIONS.md#d38-m2-fresh-per-run-home-for-claude-code-cheapest-models-for-live-tests)). The harness therefore has the same keychain access as under the real `HOME`.
- **Shared config dir.** All runs share one isolated `CLAUDE_CONFIG_DIR`; it carries older sessions and writes `.claude.json`, backups, `plugins/synced` and `skills/synced`.
- **Account state.** claude.ai connectors and account-synced plugins and skills (31 skills listed, measured) arrive from the account.
- **Managed settings.** Vendor docs say only managed settings override `--settings`. Their path per OS: unknown.
- **User shell files.** With the real `HOME` the Bash tool sources `~/.zshrc`; no environment variable stopped it (measured). Only the replaced `HOME` does, and it hides the user's git identity and ssh keys.
- **A second Claude.** A separate `claude` under another config dir is detectable only indirectly (a second project slug, vendor docs).

## 9. Quirks and open questions

- `SessionStart` never reached an `http` hook handler on 2.1.259 and the failure was silent; a `command` hook fires, sometimes twice on resume (measured, V1-era).
- `USER` must be in the environment or login reports false (measured).
- The MCP cache path is `$HOME/Library/Caches/claude-cli-nodejs`; `XDG_CACHE_HOME` is not consulted (read from the bundle, not separately measured).
- Whether another install's updater could prune 2.1.283 while an unlocked run uses it: not measured.
- Open: every "unknown" still above for Linux and Windows (a logged-in run, the config-dir default); the Linux credential store; the managed-settings paths; the update and connector hosts; how to install an exact version.
- Compiled from records dated 2026-09-03 to 2026-10-07 and the public connector; none of the runs were repeated for this sheet. The Linux facts come from the 2026-10-13 arm64 Docker run (PR #55) and the x86_64 runner smoke (PR #81).

## 10. Chat mode (measured 14 Oct 2026)

D57 runs the shipped connectors in their normal chat mode in the participant's terminal, and the participant types the prompt. `goalcup start` can host such a run (`[capture] interactive = true`); **this connector is still headless in this build (section 1):** its chat-mode recipe has no arguments at all (`args = []`, `claude` alone), and the connector schema refuses an empty `capture.args` ("must hold 1 to 64 arguments"); the daemon's and the protocol crate's own tests also load the shipped connector and expect its headless recipe. Its chat-mode data (`interactive = true`, `args = []`, identity `none`, native root `{config_home}/projects` with pattern `*/*.jsonl` and `since_run_start = true`, `skills_dir = "skills"`, `DISABLE_AUTOUPDATER` and `CLAUDE_CODE_RETRY_WATCHDOG` kept) waits for the schema to admit no arguments in an `interactive` profile and for those tests.

Measured on macOS arm64, Claude Code 2.1.283, 2026-10-14, by driving the TUI on a pseudo-terminal (120x40) in a fresh `git init` workspace, with `CLAUDE_CONFIG_DIR` the tracer's isolated home and a `HOME` holding only `Library/Keychains`. That home was **logged out** (the assistant answered "Not logged in · Please run /login"), so no model reply was seen:

- A new workspace shows "Quick safety check: Is this a project you created or one you trust?" with the cursor on **"No, exit"**. The participant must pick "Yes, I trust this folder" (Down, then Enter). Every fresh clone of a run shows it once.
- The transcript is **not** created at startup or after the trust prompt. It is created when the first message is submitted, at `projects/<cwd with / replaced by ->/<session uuid>.jsonl`.
- The typed message is a `type: "user"` record whose `message.content` is the text.
- `/exit` prints `Resume this session with: claude --resume <uuid>`.
- `claude --continue` in the same working directory reopens that directory's most recent session and **appends to the same file** (no new file); the file is written at startup, before any message. Under `since_run_start` such a file is old but written during the run, so it is recorded.
- Side effect: Claude Code created `.local/` and `Applications/` in the run's `HOME` (not in the config home).

PENDING (need a logged-in home, which the owner must restore; nobody on our side logs in for a test):

- A model reply in chat mode through the recording proxy.
- One 10-minute practice run of this connector on this Mac against staging (staging is deployed on 19 Oct), once the connector is in chat mode.
