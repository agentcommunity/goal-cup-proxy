# Claude Code CLI harness facts

Connector id: `claude-code-cli` · Facts for version: `2.1.283` · Written by: Goal Cup team · Last checked: 2026-10-12
Public disclosure: yes. Everything here comes from the public connector, the public decision log and Goal Cup's own measurements on macOS; no account data, tokens or personal paths are included.
Status: shipped connector

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64; Claude Code 2.1.283 unless another version is named); "vendor docs" means a quote that Goal Cup's research file took from Anthropic's Claude Code documentation, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. Linux and Windows were not measured for this harness, so most cells in those columns say "unknown".

## 1. Run one prompt, headless

The capture argv (code: [connectors/claude-code-cli/harness.toml](../../connectors/claude-code-cli/harness.toml)), goal appended as the single element after `--`:

```
claude -p --model haiku --effort low --permission-mode acceptEdits --permission-prompts none \
  --session-id <uuid minted before spawn> --output-format json \
  --strict-mcp-config --mcp-config '{"mcpServers":{}}' -- "<goal>"
```

- stdin is `/dev/null`; stdout is one JSON result object at exit (measured). Exit 0 after `end_turn` (measured).
- A missing or empty goal makes Claude exit 1 with `Input must be provided either through stdin or as a prompt argument when using --print` (measured).
- A write that needs approval is denied automatically and listed in the result's `permission_denials` (measured).
- `--session-id` must be a valid UUID and pins the id, the transcript file name, record contents and hook payloads (measured; vendor docs).
- Do not pass `--no-session-persistence` (it suppresses the transcript). `--bare` skips hooks and CLAUDE.md but does not use the subscription login (vendor docs).
- On transport failure Claude retries with backoff, up to 10 times by default on 2.1.283 (measured: retries 1 to 7 of 10 seen, backoff 0.6 s to 38 s). The connector sets `CLAUDE_CODE_RETRY_WATCHDOG=1`, which vendor docs say raises this for unattended sessions (code comment; not measured by Goal Cup).
- A 64 KiB goal as one argv element was safe on macOS (measured). Other OS limits: unknown.

## 2. Install and pin a version

- Installed form on macOS: `~/.local/bin/claude` is a symlink to `~/.local/share/claude/versions/2.1.283`, a single native executable (measured). Other installs' updaters move the symlink (2.1.281 to 2.1.283 in three days, measured), so Goal Cup execs and pins the versioned file by SHA-256 (code: `[capture.executable]`).
- Print the version: `claude --version` (code: V1 manifest `[detection]`).
- Install an exact version: unknown (not recorded by Goal Cup).
- Stop self-update: `DISABLE_AUTOUPDATER=1` (code: connector env).
- Version lock: while a run is going Claude creates and removes `~/.local/state/claude/locks/2.1.283.lock`; `XDG_STATE_HOME` redirects it (measured).
- Linux and Windows install locations: unknown. A pinned SHA-256 matches one platform and one architecture only (design note in goal-cup-tracer history, `docs/research/platform-portability.md`, seam 7).

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
- **Where credentials are stored.** macOS: the login Keychain, shared across config directories (code comment; consistent with a measurement: `loggedIn` was false with an empty `HOME` and true again once `HOME` held only a `Library/Keychains` link to the real keychain directory). Account metadata (`oauthAccount`, `userID`) sits in `.claude.json` (code: connector seeds these keys). Linux: unknown; Goal Cup's design note believes a credentials file under the config dir, not measured. Windows: unknown.
- **Auth needs `USER`.** `loggedIn` was false when `USER` was absent from the environment; setting only `LOGNAME` did not help (measured).
- **Files derived from the key.** None known.

## 6. Network

- **Model host.** `api.anthropic.com`, `POST /v1/messages`, HTTP/1.1, 200 `text/event-stream` (measured, 2.1.283, 2026-09-28 wire capture). Request 2 was a threaded continuation.
- **Auth host.** `platform.claude.com` (code: `auth_hosts`).
- **Connectors and sync.** At each launch Claude starts the account's claude.ai connectors and account-synced plugin MCP servers unless `ENABLE_CLAUDEAI_MCP_SERVERS=false`, `CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS=1` and `--strict-mcp-config` are set (measured; which one suffices alone was not isolated). Hosts: unknown. Plugins and skills are refreshed from the account at each launch (measured).
- **Telemetry.** OpenTelemetry export is opt-in via `CLAUDE_CODE_ENABLE_TELEMETRY=1` and `OTEL_*` (vendor docs). Any default telemetry hosts: unknown.
- **Update host.** unknown (the updater is disabled with `DISABLE_AUTOUPDATER=1`).
- **Proxy.** `HTTPS_PROXY` is honoured; a loopback forward proxy captured the exchange above (measured, 2.1.283). Transport is plain HTTP/1.1 with server-sent events; no WebSocket seen.
- **Custom CA.** `NODE_EXTRA_CA_CERTS`, extends the roots (code: CA registry). The live check was qualified on 2.1.281 first; the 2.1.283 wire capture then worked with it.
- **Base-URL overrides (Goal Cup never sets these).** `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_URL`; vendor docs say `ANTHROPIC_BASE_URL` works with a subscription login and disables Remote Control.

## 7. Process model

- **Tree.** One native executable. The Bash tool starts `zsh -c -l <script>`; MCP servers are children; subagents run in-process and write sidechain transcript files (measured).
- **Kill.** Goal Cup signals the process group, 10 s grace, then SIGKILL (code: `strategy = process_group`). SIGTERM-specific behaviour of Claude: unknown.
- **Hard kill.** `kill -9` leaves no session-end record; request body files written through the OTel raw-body switch were complete JSON up to the kill (measured, 2.1.259).
- **Exit codes.** 0 after a finished turn, 1 for a missing goal (measured). Others: unknown.

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
- Open: every "unknown" above for Linux and Windows; the Linux credential store; the managed-settings paths; the update and connector hosts; how to install an exact version.
- Compiled from records dated 2026-09-03 to 2026-10-07 and the public connector; none of the runs were repeated for this sheet.
