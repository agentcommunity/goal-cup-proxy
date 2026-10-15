# Hermes Agent harness facts

Connector id: `hermes-cli` · Facts for version: `0.19.0` · Written by: Goal Cup team · Last checked: 2026-10-12
Public disclosure: yes. Everything here comes from the public connector, Goal Cup's source reading of the installed package and one live reference row on macOS; no keys, tokens or personal paths are included.
Status: shipped connector (capture-only: Goal Cup records the run, but there is no native decoder for the session snapshot, so no own-session claim is made yet)

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64, Hermes 0.19.0, CPython 3.13.13, 2026-09-03 spike and 2026-10-02 reference row); "source" means Goal Cup read the installed 0.19.0 package without running that path; "vendor docs" means a quote that Goal Cup's research file took from Nous Research's documentation, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. Linux and Windows were not measured for this harness, so most cells in those columns say "unknown".

## 1. Run one prompt, headless

```
hermes chat -Q --provider deepseek --query=<goal>
```

(code: [connectors/hermes-cli/harness.toml](../../connectors/hermes-cli/harness.toml).)

- The goal must be one argv element spelled `--query=<goal>`. `-q -- <goal>` is refused by `argparse` ("expected one argument"), and `--query=` keeps a goal that starts with `-` from being read as an option (source).
- Hermes scans raw `sys.argv` before `argparse`: `-p`/`--profile` switches `HERMES_HOME`, and `--tui`, `--cli`, `-h` are matched, with abbreviations. Goal Cup guards these (code: `[capture.argv_guard]`).
- `-Q` prints only the final response to stdout once and prints `session_id: <id>` to stderr. The `-z` one-shot path does not emit that line and is not used (source).
- stdin is `/dev/null`. One turn, then exit; Goal Cup requires a success exit. Hermes's own exit codes on failure: unknown.
- `--provider` is the literal `deepseek`; `auto` is refused because it picks a provider from the environment (source). `--safe-mode` and `--ignore-user-config` must not be added; both skip the private config that enables the JSON snapshot (source).
- The capture environment: `HOME` and `HERMES_HOME` are fresh private directories, `TERMINAL_HOME_MODE=profile`, `DEEPSEEK_API_KEY` for the child only, `HTTPS_PROXY`, `HERMES_CA_BUNDLE`, a fixed `PATH` (code and source).

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
- **Where the id first appears.** On stderr as `session_id: <id>`, printed only after the final turn is persisted, not at session start (source; measured on the reference row). Also the snapshot file name. Compression would rotate the id mid-run, so the connector turns compression off (code).
- **Subagents.** `delegate_task` children get their own session rows with `parent_session_id` and `_delegate_from` inside `model_config` JSON (measured).
- **Model.** Per session and per model in `session_model_usage`; not per message (measured).

## 5. Auth

- **Provider and key (used by Goal Cup).** `--provider deepseek` with `DEEPSEEK_API_KEY` set on the child only ([D40](../DECISIONS.md#d40-m2-finish-keeps-the-termination-grace-the-spawn-window-is-closed-reservation-release-waits-hermes-uses-deepseek)). Hermes also supports subscription-style logins through other providers; the 2026-09-03 spike used an `openai-codex` login held in `auth.json`.
- **Credential seeding.** Hermes copied a host-level CLI's token into its isolated home with no user action, and the same code path reads two more credential files in the home directory (measured, 2026-09-03). Goal Cup therefore runs with an empty private `HOME`.
- **File derived from the key.** Without a seed, Hermes copies the environment key into a credential-pool entry in `$HERMES_HOME/auth.json` and writes `secret_fingerprint`, the first 16 hex digits of the key's SHA-256 (source). Goal Cup pre-seeds `auth.json` with `{"version": 1, "providers": {}, "suppressed_sources": {"deepseek": ["env:DEEPSEEK_API_KEY"]}}`, and after an offline run the file stayed byte-identical to the seed ([D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules); measured).
- **Key fragments on disk.** After a non-retryable 4xx, `request_dump_*.json` carries `Bearer key[:8]...key[-4:]`, and a 401 body may echo a masked key (source). Goal Cup scans for these before retaining anything.
- **Where credentials are stored.** `auth.json` and `.env` under `$HERMES_HOME` on macOS (measured). No Keychain use observed. Linux and Windows: unknown.

## 6. Network

- **Model host.** `api.deepseek.com`, `POST /v1/chat/completions`, HTTP/1.1, 200, server-sent events; three exchanges in the reference row (measured, 2026-10-02). Served model field `deepseek-flash` for a request of `deepseek-v4-flash` (measured).
- **Other hosts.** `models.dev` is asked for model metadata by a bare `requests.get`, tunnels opaquely and has no switch (source; a CONNECT to it is expected by Goal Cup's offline-key test, and the reference row recorded no chain record for it). Hermes would also download a `tirith` scanner from GitHub at start; `security.tirith_enabled: false` stops it (code and source). The 2026-09-03 spike saw a 32.8 MB binary downloaded mid-session (measured). The update-check host behind `.update_check`: unknown.
- **Telemetry.** None by default (vendor docs: the FAQ says Hermes collects none). A built-in OTLP exporter is opt-in and content-free (vendor docs).
- **Proxy.** `HTTPS_PROXY` is honoured; the reference row went through a loopback forward proxy with `NO_PROXY` for loopback only (measured).
- **Custom CA.** `HERMES_CA_BUNDLE`, checked first by Hermes's provider clients; it replaces the trust roots rather than extending them, which fails closed (source; [D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules) and code: CA registry mode `replace`). `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE` are not used by Goal Cup. `models.dev` calls keep certifi roots.
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

- No model or reasoning effort is set since 2026-10-14 (owner direction); the provider stays `deepseek` because `DEEPSEEK_API_KEY` is the one admitted credential. The reference row used `--model deepseek-v4-flash` and `agent.reasoning_effort: "none"` (which must be quoted so YAML does not read it as a boolean). Without `--model`, Hermes 0.19.0 falls back to the first entry of its DeepSeek model list, `deepseek-v4-pro`, with the provider's default thinking (source: `cli_agent_setup_mixin.py`, `models.py`; not measured).
- Title generation and compression stay off so there is one session id and no auxiliary model call (code); compression off is a real limit on very long runs. The `tirith` scanner stays off (secret review I-2), and the remote model catalogue stays off.
- The first reference row passed capture integrity but its trace has not been re-decoded; the claim stays capture-only ([reference row, D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules)).
- Open: every "unknown" above; the Linux and Windows `HERMES_HOME`; the version command; the update-check host; failure exit codes; whether the console script bytes are portable.
