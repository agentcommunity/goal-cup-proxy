# Hermes Agent harness facts

Connector id: `hermes-cli` · Facts for version: `0.19.0` · Written by: Goal Cup team · Last checked: 2026-10-16
Public disclosure: yes. Everything here comes from the public connector, Goal Cup's source reading of the installed package and one live reference row on macOS; no keys, tokens or personal paths are included.
Status: shipped connector (capture-only: Goal Cup records the run, but there is no native decoder for the session snapshot, so no own-session claim is made yet)

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64, Hermes 0.19.0, CPython 3.13.13, 2026-09-03 spike and 2026-10-02 reference row); "source" means Goal Cup read the installed 0.19.0 package without running that path; "vendor docs" means a quote that Goal Cup's research file took from Nous Research's documentation, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. Linux and Windows were not measured for this harness, so most cells in those columns say "unknown".

## 1. Run one prompt, in chat mode first

Chat mode (what the connector runs, D57, D61; measured 2026-10-16 on a pseudo-terminal with an empty `HOME` and `HERMES_HOME`):

```
hermes chat
```

(code: [connectors/hermes-cli/harness.toml](../../connectors/hermes-cli/harness.toml).)

- `hermes chat` (and a bare `hermes`) opens the classic prompt_toolkit REPL. The Ink TUI needs `--tui`, `HERMES_TUI=1` or `display.interface: tui` in `config.yaml`; the connector passes none of them, and the TUI writes the same session file if the participant chose it (measured).
- The participant types the prompt at the `❯` prompt. `/exit`, `/quit` or Ctrl-D at an empty prompt end Hermes with exit 0 (measured). Ctrl-C at the setup question exits 1 (measured).
- First launch in a home with no provider: Hermes prints `It looks like Hermes isn't configured yet -- no API keys or providers found.` and asks `Run setup now? [Y/n]`. Enter or `y` runs its own setup wizard (Quick Setup with Nous Portal, Full setup, or Blank Slate; arrow keys and Enter). `n` exits 1. `goalcup prep` opens Hermes there, so the participant answers it once (measured).
- The banner prints `Session: <id>` on the terminal. The tracer does not read the terminal: the connector's identity is `none`, and the run's sessions are the snapshots written since it started (D57).
- `hermes chat --continue` resumes the most recent session (`↻ Resumed session <id>`); it is the connector's `resume_args` for `goalcup continue` (measured).
- The session file needs `sessions.write_json_snapshots: true`, which has no environment variable and is off by default. The connector's first `configure` command, `hermes config set sessions.write_json_snapshots true`, writes that one key into `config.yaml` (output `✓ Set sessions.write_json_snapshots = True in <HERMES_HOME>/config.yaml`, exit 0, other keys kept; measured). `goalcup prep` runs it after the participant's setup, and `goalcup start` before each launch (D61).
- The session file keeps changing after a compaction only with `compression.in_place: false` (default `true`, `hermes_cli/config.py:1515`; read at `agent/agent_init.py:1667`). In place, a compaction keeps the id with fewer messages and `_save_session_log` never overwrites a snapshot that holds more (`run_agent.py:2754`), so the file freezes. Off, the compaction starts a new id (`agent/conversation_compression.py:1247`) and the next save writes `session_<new id>.json` (`run_agent.py:2723`). The connector's second `configure` command, `hermes config set compression.in_place false`, sets it (exit 0, measured). It changes how the session is stored, not whether Hermes compacts (D61).
- Setup closes Hermes: after the first-run wizard it exits 0 and opens no chat (`hermes_cli/main.py:2521-2523`). `hermes chat --continue` in a home with no session prints `No previous CLI session found to continue.` and exits 1 (measured), so the way on after a setup is `goalcup prep` again, then the run.
- Hermes scans raw `sys.argv` before `argparse`: `-p`/`--profile` switches `HERMES_HOME`, and `--tui`, `--cli`, `-h` are matched, with abbreviations. Goal Cup guards these (code: `[capture.argv_guard]`).
- The capture environment: `HOME` is a fresh private directory, `HERMES_HOME` the shared Goal Cup home `<cfg>/capture-homes/hermes-cli`, `TERMINAL_HOME_MODE=profile`, the participant's `PATH`, `TERM`, `COLORTERM`, `LANG`, `SHELL` and `LC_*`, `HTTPS_PROXY` and `HERMES_CA_BUNDLE` (code). No provider key: Hermes reads its own `.env`.

Headless (the recipe before D61, frozen at [`headless-connectors/hermes-cli`](../../crates/goalcup-protocol/tests/fixtures/headless-connectors/hermes-cli/harness.toml) for the tests of that recipe): `hermes chat -Q --provider deepseek --query=<goal>`, with the key passed to the child, a seeded per-run home and the `session_id: <id>` line on stderr as identity.

## 2. Install and record a version

- Distribution: the Python package `hermes-agent`, installed with `uv tool install hermes-agent` (code: `harnesses/install.sh`, which installs the latest version). `hermes-agent==0.19.0` is the measured version. The `==` spelling is standard uv/pip syntax, not run for this sheet. No build is pinned (D53), and the connector has no `version_args` because the version command is not known.
- Requires Python `>=3.11,<3.14`; measured with CPython 3.13.13 (source; metadata).
- Binary: `$UV_TOOL_BIN_DIR/hermes` is a 372-byte console script whose entry point is `hermes_cli.main:main`. The daemon records its SHA-256 (D53). Whether those bytes are the same on every install, OS and interpreter path: unknown.
- Print the version: unknown (the connector does not use a version command; the installed distribution name and version are read from the package metadata).
- Linux and Windows install locations: unknown. A Docker image `nousresearch/hermes-agent` and a desktop app for macOS, Windows and Linux exist (vendor docs); neither was measured.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HERMES_HOME`, default `~/.hermes` (vendor docs; measured with an isolated dir). Holds `config.yaml`, `auth.json` (mode 0600), `.env`, `.update_check`, `SOUL.md`, `plugins/`, `hooks/`, `skills/` | unknown (vendor docs name `~/.hermes`; Docker uses `HERMES_HOME=/opt/data`; not measured on Linux) | unknown |
| Session / transcript store | `$HERMES_HOME/state.db`, SQLite in WAL mode, schema version 22 on 0.19.0 (measured). Per-run snapshot `$HERMES_HOME/sessions/session_<id>.json`, written only when `sessions.write_json_snapshots: true` (default false; source and measured). `request_dump_<sid>_<ts>.json` also lands in `sessions/` after a non-retryable 4xx (source) | unknown | unknown |
| Logs / cache | `$HERMES_HOME/logs/` (`agent.log`, redacted, rotates at 5 MiB times 3: vendor docs); `$HERMES_HOME/cache/delegation/live/<id>/task-<n>.log` and `manifest.json`, pruned after 7 days (measured file layout; docs for pruning); `session-exports/` for `hermes sessions export` (vendor docs) | unknown | unknown |
| Env vars that move these | `HERMES_HOME` moves all of the above (measured). `TERMINAL_HOME_MODE=profile` makes tool children use `$HERMES_HOME/home` as their home (source). `HOME` still matters: Hermes reads host-level credential files in the home directory (measured, 2026-09-03). `-p`/`--profile` switches `HERMES_HOME` (source) | same variable names expected; behaviour not measured on Linux | unknown |

## 4. Session format and identity

- **State store.** `state.db` holds every session from every surface: `sessions` and `messages` tables with an `AUTOINCREMENT` message id, plus `session_model_usage` and `async_delegations` (measured on 0.19.0, schema 22). Compaction soft-archives rows (`active=0`, `compacted=1`) rather than deleting them; `hermes sessions delete/prune` hard-deletes (vendor docs).
- **Snapshot Goal Cup reads.** `session_<id>.json` is one JSON object per session (`session_id`, `model`, `base_url`, `platform`, times, normalized system prompt, tools, messages), rewritten whole at every persistence point with a temp file, `fsync` and replace (source). It is a normalized, secret-redacted final state, not an event stream, and omits per-message timestamps and token counts (source). Goal Cup has no decoder for it yet.
- **One per session.** One snapshot file per session id; a resume appends to the same `sessions` row (measured, spike).
- **Id.** `YYYYMMDD_HHMMSS_<six lowercase hex>`, generated at start (source; the reference row had `20261002_125737_dc0697`).
- **Where the id first appears.** In chat mode, in the banner (`Session: <id>`) on the terminal and, after a message, as the snapshot's file name (measured 2026-10-16). Headless with `-Q`, on stderr as `session_id: <id>` after the final turn (source; measured on the reference row). Compression keeps the id in place by default; the connector sets `compression.in_place: false`, so each compaction starts a new id and a new snapshot, written since the run started and recorded (D61).
- **Subagents.** `delegate_task` children get their own session rows with `parent_session_id` and `_delegate_from` inside `model_config` JSON (measured).
- **Model.** Per session and per model in `session_model_usage`; not per message (measured).

## 5. Auth

- **The participant's own setup (D61).** The tracer passes no provider key. Hermes's setup wizard (opened by `goalcup prep`, or `hermes setup` / `hermes model` in that home) keeps the provider and model in `$HERMES_HOME/config.yaml` (`model.provider`, `model.default`, `model.base_url`) and an API key in `$HERMES_HOME/.env` (mode 0600). `hermes chat` loads `.env` itself, over a same-named variable in its environment (source). OAuth providers (Nous Portal and others) keep their tokens in `auth.json` (source).
- **What the tracer records of them.** Nothing of their content. The native source reads `sessions/session_*.json` only. The uploaded baseline lists `.env` and `auth.json` by name, size and SHA-256, as every file of the harness home (D56).
- **File derived from the key.** Hermes copies an environment or `.env` key into a credential-pool entry in `auth.json` with `secret_fingerprint`, the first 16 hex digits of the key's SHA-256, and no raw key (source; the pre-D61 seed suppressed this).
- **Key fragments on disk.** After a failed request, `sessions/request_dump_*.json` carries `key[:8]` (measured 2026-10-16 with a synthetic key). The native pattern never matches it. A key the agent prints into the session is removed by the daemon's shape rules when printed raw or JSON-escaped, and not when encoded ([CHEAT-CHECKLIST](../CHEAT-CHECKLIST.md#provider-keys-d61-what-the-tracer-does-not-hold)).
- **Credential seeding from the host.** Hermes copied a host-level CLI's token into its isolated home with no user action, and the same code path reads two more credential files in the home directory (measured, 2026-09-03). Goal Cup therefore runs with an empty private `HOME`.
- **Where credentials are stored.** `auth.json` and `.env` under `$HERMES_HOME` on macOS (measured). No Keychain use observed. Linux and Windows: unknown.

## 6. Network

- **Model host.** `api.deepseek.com`, `POST /v1/chat/completions`, HTTP/1.1, 200, server-sent events; three exchanges in the reference row (measured, 2026-10-02). Served model field `deepseek-flash` for a request of `deepseek-v4-flash` (measured).
- **Other hosts.** `models.dev` is asked for model metadata by a bare `requests.get`, tunnels opaquely and has no switch (source; the reference row recorded no chain record for it). Hermes also downloads a `tirith` scanner from GitHub at start unless the participant's `security.tirith_enabled: false` stops it (source; the pre-D61 seed set it). It tunnels opaquely like any host not in `capture.hosts`. The 2026-09-03 spike saw a 32.8 MB binary downloaded mid-session (measured). The update-check host behind `.update_check`: unknown.
- **Telemetry.** None by default (vendor docs: the FAQ says Hermes collects none). A built-in OTLP exporter is opt-in and content-free (vendor docs).
- **Proxy.** `HTTPS_PROXY` is honoured; the reference row went through a loopback forward proxy with `NO_PROXY` for loopback only (measured).
- **Custom CA.** `HERMES_CA_BUNDLE`, checked first by Hermes's provider clients; it replaces the trust roots rather than extending them, which fails closed (source; [D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules) and connector `capture.ca`, mode `replace`). `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE` are not used by Goal Cup. `models.dev` calls keep certifi roots.
- **WebSocket / SSE.** SSE on the provider call (measured). `hermes serve` and the dashboard use WebSocket (vendor docs); not used.
- **Base-URL overrides (Goal Cup never sets these).** `DEEPSEEK_BASE_URL`, `OPENAI_BASE_URL`, config `model.base_url`, `HERMES_INFERENCE_PROVIDER` (source).

## 7. Process model

- **Tree.** A Python process (`hermes_cli.main`). Tool processes are started with `start_new_session=True`, each a session leader of its own (source, `tools/environments/local.py`). `delegate_task` runs subagents in-process with their own session rows.
- **Kill.** Goal Cup signals the process group, 10 s grace, then SIGKILL (code: `strategy = process_group`). A tool process the run left behind is outside the root's walk once its parent is gone and is not claimed by `Confirmed` ([TERMINATION](../TERMINATION.md)). Hermes's own SIGTERM behaviour: unknown.
- **Exit codes.** Success is required; failure codes: unknown.
- **Hooks and plugins.** 23 hook names on 0.19.0; a plugin allow-list key exists only in the home-level config, and `--safe-mode` skips plugin discovery entirely (measured; relevant only to hook-based capture, which the Hermes connector does not use).

## 8. Things Goal Cup cannot isolate

- **`models.dev` egress.** Carries neither the key nor session content; accepted and recorded as non-model traffic (code comment; D41).
- **Server side.** DeepSeek's routing: a request for `deepseek-v4-flash` is served as `deepseek-flash` (measured). Hermes rewrites `deepseek-flash` to `deepseek-chat` in 0.19.0 (D41).
- **Host credentials.** Isolation depends on the empty private `HOME` and `HERMES_HOME`; Hermes's own code reads host credential files from the home directory (measured).
- **Orphaned tool processes.** See section 7.
- **Machine-wide or managed settings.** unknown.
- **Another Hermes.** A second `HERMES_HOME`, `--profile`, a Docker container or a desktop app pointed at a remote `hermes serve` is invisible to a local tracker (vendor docs).

## 9. Quirks and open questions

- No model, provider or reasoning effort is set (owner direction 2026-10-14, D61): Hermes uses what the participant chose in its setup.
- Since D61 the home is not seeded, so Hermes's defaults apply: title generation (an auxiliary model call), compression, the `tirith` command scanner (downloaded from GitHub at start; with the network refused it prints `tirith security scanner enabled but not available`, measured) and the remote model catalogue. The connector's `configure` commands set only how the session is stored: `sessions.write_json_snapshots: true` and `compression.in_place: false` (D61).
- Only the DeepSeek host is recorded (`providers = ["deepseek"]`), and `HERMES_CA_BUNDLE` makes Hermes trust only the proxy, so a participant who picks another service gets failing model calls until the connector reads the model host from Hermes's own config.
- The first reference row passed capture integrity but its trace has not been re-decoded; the claim stays capture-only ([reference row, D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules)).
- Open: every "unknown" above; the Linux and Windows `HERMES_HOME`; the update-check host; failure exit codes; whether the console script bytes are portable; whether `hermes profile use` inside the shared home redirects runs (the `active_profile` file is not guarded).
