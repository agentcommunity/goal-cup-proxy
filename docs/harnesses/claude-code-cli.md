# Claude Code harness facts

Connector id: `claude-code-cli` · Facts for version: `2.1.312` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Anthropic's public Claude Code docs, the npm registry, and Goal Cup runs on macOS and Linux in September and October 2026. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/claude-code-cli.toml`](../../connectors/claude-code-cli.toml))

Source words: "docs" is [code.claude.com/docs](https://code.claude.com/docs/en/network-config) (network-config and claude-directory pages) as read on 2026-10-31; "measured" is a Goal Cup run on 2.1.283 to 2.1.312; "unknown" is not measured or read. The newest build in the registry on 29 Oct was 2.1.312, and releases come about one a weekday.

## 1. Run one prompt, in chat mode first

- `claude` with no argument opens the chat. The participant types the prompt; `/exit` quits and prints a `claude --resume <id>` line. `claude --continue` reopens the most recent session of the working folder and appends to the same transcript file (measured). The connector's `resume` is `["--continue"]`.
- A new folder shows "Is this a project you trust?" with the cursor on "No, exit" (measured). The participant answers it; the connector's `first_screen` line says so.
- `claude -p "<prompt>"` is the one-shot form; it is not chat mode and is not used. Do not add `--no-session-persistence` or set `CLAUDE_CODE_SKIP_PROMPT_HISTORY`: both stop the transcript ([RULES](../RULES.md)).

## 2. Install and record a version

- `curl -fsSL https://claude.ai/install.sh | bash` (a version can follow as `bash -s 2.1.312`), or the npm package `@anthropic-ai/claude-code`, which pulls the same native executable ([docs](https://code.claude.com/docs/en/setup)). On macOS the install is a symlink in `~/.local/bin` to a file under `~/.local/share/claude/versions/` (measured).
- `claude --version` prints `<version> (Claude Code)` (measured). The connector sets no `version` word, so any build runs and the `start` event records the one that ran.
- The updater is off with `DISABLE_AUTOUPDATER=1`; `DISABLE_UPDATES=1` also blocks `claude update` (docs). The connector sets both.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$CLAUDE_CONFIG_DIR`, default `~/.claude`; `.claude.json` sits beside it, or at `~/.claude.json` without the variable (docs; measured) | same (docs; a logged-in Linux run was not measured) | `%USERPROFILE%\.claude` (docs) |
| Session store | `projects/<project>/<session>.jsonl`; `<project>` is the working folder with every character other than a letter or digit replaced by `-` (docs; measured) | same | same |
| Subagents and spills | `projects/<project>/<session>/subagents/agent-<id>.jsonl` and `.meta.json`; workflow agents one level deeper, under `subagents/workflows/<workflow>/`; large tool output in `tool-results/` (docs; measured) | same | same |
| Logs / cache | `debug/` while `--debug` is on, `shell-snapshots/` (removed at a clean exit), `session-env/` (docs); the MCP log cache is under `~/Library/Caches/claude-cli-nodejs` (measured) | unknown | unknown |
| Env vars that move these | `CLAUDE_CONFIG_DIR` moves everything above; the MCP cache follows `HOME` (measured) | same | unknown |

The connector sets `CLAUDE_CONFIG_DIR` to the isolated home and leaves `HOME` alone. Transcripts are deleted after `cleanupPeriodDays`, 30 by default (docs); a week-long run is inside that.

## 4. Session format and identity

- JSONL, one file per session, mostly append-only. Records carry `uuid`, `parentUuid`, `isSidechain`, `sessionId`, `timestamp`, `cwd` and `version`; the model is on each assistant record (measured). Since 2.1.300 each request's system prompt, tool definitions and parameters are written too (docs). Anthropic calls the format internal; it changes between versions.
- Timestamps are not monotonic in file order. A set-aside file is named `<session>.jsonl.superseded-<timestamp>` or `<session>.orphaned-<timestamp>-<suffix>.jsonl` (docs). The connector reads every `*.jsonl` under `projects` as it grows, and copies `*.meta.json` and `tool-results/*` as snapshots; the last two may match nothing, which is not a warning.
- The session id is a UUID. It is not known at launch: the file appears when the first message is submitted (measured), and the id is in the file name and every record. Resuming in two terminals interleaves them in one file (docs).

## 5. Auth

- Sign-in with a claude.ai account (OAuth), `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, an `apiKeyHelper`, or a cloud provider's credentials ([docs](https://code.claude.com/docs/en/authentication)). Chat mode passes no key: the participant signs in once, in `goalcup setup`.
- Does the harness have a variable that moves its config and login? Yes, `CLAUDE_CONFIG_DIR`; the connector sets it. On Linux the OAuth tokens are in `$CLAUDE_CONFIG_DIR/.credentials.json`, which the connector lists as a secret file. On macOS the login is in the user's Keychain, which no variable moves and which every config folder shares (measured: `loggedIn` was false under an empty `HOME`). Windows: unknown.
- Login needs `USER` in the environment; `LOGNAME` alone was not enough (measured).

## 6. Network

- Proxy: `https_proxy`, `HTTPS_PROXY`, `http_proxy`, `HTTP_PROXY`, the first one set; `NO_PROXY` is honoured; no SOCKS ([docs](https://code.claude.com/docs/en/network-config)). Model calls are HTTP/1.1 server-sent events, no WebSocket (measured). It is a Bun executable, not Node: `NODE_USE_ENV_PROXY` does nothing. A proxy value without a scheme stops the launch (docs).
- CA: `NODE_EXTRA_CA_CERTS` adds to the roots (docs). The default trust is the bundled Mozilla set plus the OS store, and `CLAUDE_CODE_CERT_STORE` narrows it (docs); the connector unsets it. A name-constrained run CA was accepted on 2.1.312 (measured).
- Settings `env` block: any variable on the network-config page can be set in `settings.json`, in the config folder or in the project's `.claude/settings.json` or `settings.local.json` (docs). Whether it beats the launch environment is not measured, so the connector refuses to start if one of those three files sets a proxy, `NO_PROXY`, `CLAUDE_CODE_CERT_STORE` or CA value.
- Hosts: `api.anthropic.com` for model calls, or a gateway or cloud provider when `ANTHROPIC_BASE_URL` or its Bedrock, Vertex, Foundry and AWS variants are set; the connector hints those variables and the `env` keys in the three settings files. `platform.claude.com`, `claude.ai` and `sso.claude.com` carry sign-in; `mcp-proxy.anthropic.com` carries the account's claude.ai connectors; two Datadog hosts carry optional telemetry (docs).
- Account connectors and synced plugins start at each launch, and their traffic passes as a tunnel, not a record. Which switch alone stops them was not isolated (measured).

## 7. Process model

- One native executable. Subagents (the Agent tool) run in the same process, up to 20 at once, nested 3 deep, and share its environment (docs). Workflow agents (`/deep-research`, dynamic workflows) run many more; whether they are separate processes is not verified (docs).
- The Bash tool starts the user's shell as a login shell, so the shell's startup files run (measured). MCP servers are children.
- Exit codes: 1 for a missing prompt in `-p` mode and 1 with no login (measured); others unknown.

## 8. Things Goal Cup cannot isolate

- The macOS Keychain and the account's `~/.ssh`: the connector does not move `HOME`, so the harness has the access the participant has.
- Account state: claude.ai connectors, synced plugins and skills arrive from the account at each launch.
- Managed settings override every settings file; their path per OS: unknown.
- Background agents (`claude agents`, `--bg`, `/background`) run under a per-user supervisor that outlives the shell and keeps the environment of whichever shell started it first (docs). The connector sets `CLAUDE_CODE_DISABLE_AGENT_VIEW=1`; a supervisor started earlier outside goalcup is not under that control.
- A second `claude` under another config folder shows only as a second project folder.

## 9. Quirks and open questions

- An `http` hook handler never received `SessionStart` on 2.1.259 (measured); the connector uses no hooks.
- Open: a logged-in Linux run, the Linux cache paths, the managed-settings paths, whether workflow agents are separate processes, and an exact-version install other than the installer's argument.

## Run without permission prompts

Claude Code asks before it runs a command or writes outside its rules, and a run allows only "keep going". The connector's `no_prompts` rows write two keys into `settings.json` in the isolated home: `permissions.defaultMode = "bypassPermissions"` and `skipDangerousModePermissionPrompt = true`. Without the second key, 2.1.283 and 2.1.299 start in `default` mode ("Permission mode downgraded to default") and ask (measured). Claude Code refuses bypass mode as root unless `IS_SANDBOX=1`; run it as a normal user (docs). See [HARNESS-SETUP](../HARNESS-SETUP.md#claude-code).

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `PAD-RESULTS.md`; this sheet does not copy them.
