# Hermes Agent harness facts

Connector id: `hermes-cli` · Facts for version: `0.19.0` · Written by: Goal Cup team · Last checked: 2026-10-16
Public disclosure: yes. Everything here comes from the public connector, Goal Cup's source reading of the installed package and one live reference row on macOS; no keys, tokens or personal paths are included.
Status: shipped connector. Its native session snapshot is capture-only, so it supports no own-session
claim yet. The `openai-chat-completions` wire decoder has been measured on retrieved Hermes traffic;
the historical reference row below remains explicitly not re-decoded.

Source words used in every cell: "measured" means Goal Cup ran it (macOS 26, arm64, Hermes 0.19.0, CPython 3.13.13, 2026-09-03 spike and 2026-10-02 reference row); "source" means Goal Cup read the installed 0.19.0 package without running that path; "vendor docs" means a quote that Goal Cup's research file took from Nous Research's documentation, not re-fetched for this sheet; "code" means the Goal Cup repository; "unknown" means nobody on our side has measured or read it. The later 17 Oct macOS/Linux staging practices passed ([current WP0 measurements](../../TODO.md#wp0-measure)); that does not fill every platform/auth cell below. Windows and the longer/interruption test matrix remain pending.

## 1. Run one prompt, in chat mode first

Chat mode (what the connector runs, D57, D61; measured 2026-10-16 on a pseudo-terminal with an empty `HOME` and `HERMES_HOME`):

```
hermes chat
```

(code: [connectors/hermes-cli/harness.toml](../../connectors/hermes-cli/harness.toml).)

- `hermes chat` (and a bare `hermes`) opens the classic prompt_toolkit REPL. The Ink TUI needs `--tui`, `HERMES_TUI=1` or `display.interface: tui` in `config.yaml`; the connector passes none of them, and the TUI writes the same session file if the participant chose it (measured).
- The participant types the prompt at the `❯` prompt. `/exit`, `/quit` or Ctrl-D at an empty prompt end Hermes with exit 0 (measured). Ctrl-C at the setup question exits 1 (measured).
- First launch in a home with no provider: Hermes prints `It looks like Hermes isn't configured yet -- no API keys or providers found.` and asks `Run setup now? [Y/n]`. Enter or `y` runs its own setup wizard (Quick Setup with Nous Portal, Full setup, or Blank Slate; arrow keys and Enter). `n` exits 1. `goalcup setup` opens Hermes there, so the participant answers it once (measured).
- The banner prints `Session: <id>` on the terminal. Goal Cup Proxy does not read the terminal: the connector's identity is `none`, and the run's sessions are the snapshots written since it started (D57).
- `hermes chat --continue` resumes the most recent session (`↻ Resumed session <id>`); it is the connector's `resume_args` for `goalcup continue` (measured).
- The session file needs `sessions.write_json_snapshots: true`, which has no environment variable and is off by default. The connector's first `configure` command, `hermes config set sessions.write_json_snapshots true`, writes that one key into `config.yaml` (output `✓ Set sessions.write_json_snapshots = True in <HERMES_HOME>/config.yaml`, exit 0, other keys kept; measured). `goalcup setup` runs it after the participant's setup, and `goalcup start` before each launch (D61).
- The session file keeps changing after a compaction only with `compression.in_place: false` (default `true`, `hermes_cli/config.py:1515`; read at `agent/agent_init.py:1667`). In place, a compaction keeps the id with fewer messages and `_save_session_log` never overwrites a snapshot that holds more (`run_agent.py:2754`), so the file freezes. Off, the compaction starts a new id (`agent/conversation_compression.py:1247`) and the next save writes `session_<new id>.json` (`run_agent.py:2723`). The connector's second `configure` command, `hermes config set compression.in_place false`, sets it (exit 0, measured). It changes how the session is stored, not whether Hermes compacts (D61).
- Setup closes Hermes: after the first-run wizard it exits 0 and opens no chat (`hermes_cli/main.py:2521-2523`). `hermes chat --continue` in a home with no session prints `No previous CLI session found to continue.` and exits 1 (measured). Such a run has no session file yet, so `goalcup continue` opens `hermes chat` without `--continue`, and the setup stays in the home: the way on after a setup in a run is `goalcup continue`.
- Hermes scans raw `sys.argv` before `argparse`: `-p`/`--profile` switches `HERMES_HOME`, and `--tui`, `--cli`, `-h` are matched, with abbreviations. Goal Cup guards these (code: `[capture.argv_guard]`).
- The capture environment: `HOME` is a fresh private directory, `HERMES_HOME` the shared Goal Cup home `<cfg>/capture-homes/hermes-cli`, `TERMINAL_HOME_MODE=profile`, the participant's `PATH`, `TERM`, `COLORTERM`, `LANG`, `SHELL` and `LC_*`, `HTTPS_PROXY` and `HERMES_CA_BUNDLE` (code). No provider key: Hermes reads its own `.env`.

Headless (the recipe before D61, frozen at [`frozen-connectors/hermes-cli`](../../crates/goalcup-protocol/tests/fixtures/frozen-connectors/hermes-cli/harness.toml) for the tests of that recipe): `hermes chat -Q --provider deepseek --query=<goal>`, with the key passed to the child, a seeded per-run home and the `session_id: <id>` line on stderr as identity.

## 2. Install and record a version

- Distribution: the Python package `hermes-agent`. Goal Cup installs Hermes with `uv tool install hermes-agent==0.19.0` (see [Hermes setup](../HARNESS-SETUP.md#hermes)). PyPI's newest `hermes-agent` is 0.19.0 (20 Jul 2026); later releases (0.20 to 0.22) exist only as git tags (checked 22 Oct 2026). Hermes 0.21.0 (tag `v2026.8.31`) still writes the session file Goal Cup records; 0.21.1 (tag `v2026.9.7`) removed it (section 4). Hermes's own installer installs the newest release (`v0.22.4` on 21 Oct 2026, `v0.22.5` measured on 26 Oct); since D72 that is fine, with the limits in [Hermes 0.22](#hermes-022-the-gateway-and-certificate-trust). `harnesses/install.sh`, a research script, installs the latest PyPI version unpinned, and its `git clone --depth 1` fallback gets the source's `main`. `hermes-agent==0.19.0` is the measured version. The `==` spelling is standard uv/pip syntax, not run for this sheet. No build is pinned (D53). The connector's `version_args` is `--version`: `hermes --version` prints `Hermes Agent v0.19.0 (2026.7.20)` in under a second with a fresh HOME (measured 22 Oct 2026, macOS and Docker).
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
- **Snapshot Goal Cup reads.** `session_<id>.json` is one JSON object per session (`session_id`, `model`, `base_url`, `platform`, times, normalized system prompt, tools, messages), rewritten whole at every persistence point with a temp file, `fsync` and replace (source). It is a normalized, secret-redacted final state, not an event stream, and omits per-message timestamps and token counts (source). It is written with `indent=2`, so it is multi-line JSON, not JSON lines, and carries `message_count` (source, `run_agent.py:2694-2785` at tag `v2026.7.20`, the 0.19.0 release). It names no parent session. Goal Cup has no decoder for it yet; a snapshot stream has no protocol hint (connector schema 1 allows a hint on `append_only` stores only), so no decoder can select it.
- **Removed upstream (source, public repository, not measured).** The snapshot was always opt-in (default off); Goal Cup gets it only because the connector sets `sessions.write_json_snapshots: true`. Hermes 0.21.1 (tag `v2026.9.7`) removed the opt-in snapshot: that setting is now ignored, `state.db` is the only store, and `/save json <path>` remains as an explicit export (`tests/agent/test_session_snapshot_removal.py`). 0.21.0 (tag `v2026.8.31`) still writes it. A run on 0.21.1 or later records no native file.
- **Which version a participant gets (source, checked 22 Oct 2026, not measured).** Hermes's own README installs with `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash`. That installer clones the repository and, by default (`--channel stable`), checks out the latest stable release tag: `v0.22.4` (21 Oct 2026), which writes no snapshot. So a participant who follows Hermes's documentation records no native file. `uv tool install hermes-agent` and `pip install hermes-agent==<v>` (`scripts/linux-matrix.sh`) reach only PyPI, which stops at 0.19.0; 0.21.0 or any later release can be installed only from its git tag (or the installer's `--commit`). Decided since D72: the latest release is fine, and Goal Cup records its model calls ([Hermes 0.22](#hermes-022-the-gateway-and-certificate-trust), HARNESS-SETUP).
- **One per session.** One snapshot file per session id; a resume appends to the same `sessions` row (measured, spike).
- **Id.** `YYYYMMDD_HHMMSS_<six lowercase hex>`, generated at start (source; the reference row had `20261002_125737_dc0697`).
- **Where the id first appears.** In chat mode, in the banner (`Session: <id>`) on the terminal and, after a message, as the snapshot's file name (measured 2026-10-16). Headless with `-Q`, on stderr as `session_id: <id>` after the final turn (source; measured on the reference row). Compression keeps the id in place by default; the connector sets `compression.in_place: false`, so each compaction starts a new id and a new snapshot, written since the run started and recorded (D61).
- **Subagents.** `delegate_task` children get their own session rows with `parent_session_id` and `_delegate_from` inside `model_config` JSON (measured). In a recorded run their model calls are on the wire leg only, and the research report does not say which calls are a child's: no decoder reads Hermes's native `session_*.json` yet (measured 22 Oct: two children, on the wire, unattributed). A decoder of the snapshot alone could not attribute them either: each child is its own agent and, with the setting on, writes its own `session_<child id>.json` (source, `agent/agent_init.py:1306`, not measured), but neither that file nor the parent's `delegate_task` result (entries by `task_index`, with `summary` and `status`) names the other's session id (source, `tools/delegate_tool.py`). The link lives only in `state.db` (`parent_session_id`), which is not captured.
- **Model.** Per session and per model in `session_model_usage`; not per message (measured).

## 5. Auth

- **The participant's own setup (D61).** Goal Cup Proxy passes no provider key. Hermes's setup wizard (opened by `goalcup setup`, or `hermes setup` / `hermes model` in that home) keeps the provider and model in `$HERMES_HOME/config.yaml` (`model.provider`, `model.default`, `model.base_url`) and an API key in `$HERMES_HOME/.env` (mode 0600). `hermes chat` loads `.env` itself, over a same-named variable in its environment (source). OAuth providers (Nous Portal and others) keep their tokens in `auth.json` (source).
- **What Goal Cup Proxy records of them.** Nothing of their content. The native source reads `sessions/session_*.json` only. The uploaded baseline lists `auth.json` by name, size and SHA-256, as every file of the harness home (D56), and `.env` by name and size only: a key file is never hashed (D71).
- **File derived from the key.** Hermes copies an environment or `.env` key into a credential-pool entry in `auth.json` with `secret_fingerprint`, the first 16 hex digits of the key's SHA-256, and no raw key (source; the pre-D61 seed suppressed this).
- **Key fragments on disk.** After a failed request, `sessions/request_dump_*.json` carries `key[:8]` (measured 2026-10-16 with a synthetic key). The native pattern never matches it. A key the agent prints into the session is removed by the daemon's shape rules when printed raw or JSON-escaped, and not when encoded ([CHEAT-CHECKLIST](../CHEAT-CHECKLIST.md#provider-keys-d61-what-goal-cup-proxy-does-not-hold)).
- **Credential seeding from the host.** Hermes copied a host-level CLI's token into its isolated home with no user action, and the same code path reads two more credential files in the home directory (measured, 2026-09-03). Goal Cup therefore runs with an empty private `HOME`.
- **Where credentials are stored.** `auth.json` and `.env` under `$HERMES_HOME` on macOS (measured). No Keychain use observed. Linux and Windows: unknown.

## 6. Network

- **Model host.** Whatever `config.yaml` `model.base_url` names (D62). Measured 17 Oct 2026 with `model.base_url: https://api.deepseek.com/v1` and a made-up key, the connector listing no host: one intercepted `POST /v1/chat/completions` to `api.deepseek.com` was recorded with the `openai-chat-completions` hint and its 401 answer; `proxy admission: accepted=14 intercepted=1 tunneled_not_approved=8 failed=5 client_tls=5` (the five `client_tls` are traced, none is a model call: four are `agent/model_metadata.py` `detect_local_server_type` probes of the base host for a local server (`/api/v1/models`, `/api/tags`, `/v1/props`, `/version`, a bare `httpx.Client` with certifi roots), and one is `providers/base.py:226` `fetch_models`, a urllib `GET <base>/models` with the system roots that fills the `/model` picker; the probe then answers "not a local server", as against any hosted service, and the picker falls back to its own list. The chat client and the auxiliary client (titles, compression) trust `HERMES_CA_BUNDLE` and are recorded. No connector change: a shared CA row is only for a harness without its own variable, D43 rule 3). Earlier reference row: `api.deepseek.com`, `POST /v1/chat/completions`, HTTP/1.1, 200, server-sent events; three exchanges in the reference row (measured, 2026-10-02). Served model field `deepseek-flash` for a request of `deepseek-v4-flash` (measured).
- **Other hosts.** `models.dev` is asked for model metadata by a bare `requests.get`, tunnels opaquely and has no switch (source; the reference row recorded no chain record for it). Hermes also downloads a `tirith` scanner from GitHub at start unless the participant's `security.tirith_enabled: false` stops it (source; the pre-D61 seed set it). It tunnels opaquely like any host not in `capture.hosts`. The 2026-09-03 spike saw a 32.8 MB binary downloaded mid-session (measured). The update-check host behind `.update_check`: unknown.
- **Telemetry.** None by default (vendor docs: the FAQ says Hermes collects none). A built-in OTLP exporter is opt-in and content-free (vendor docs).
- **Proxy.** `HTTPS_PROXY` is honoured; the reference row went through a loopback forward proxy with `NO_PROXY` for loopback only (measured).
- **Custom CA.** On Hermes 0.22, a provider entry's `ssl_ca_cert`, which goalcup sets before each launch ([Hermes 0.22](#hermes-022-the-gateway-and-certificate-trust), D72). Up to 0.21.0, `HERMES_CA_BUNDLE`, checked first by Hermes's provider clients; it replaces the trust roots rather than extending them, which fails closed (source; [D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules) and connector `capture.ca`, mode `replace`). `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE` are not used by Goal Cup. `models.dev` calls keep certifi roots.
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
- **Another Hermes.** A second `HERMES_HOME`, `--profile`, a Docker container or a desktop app pointed at a remote `hermes serve` is invisible to a local Goal Cup Proxy (vendor docs).

## 9. Quirks and open questions

- No model, provider or reasoning effort is set (owner direction 2026-10-14, D61): Hermes uses what the participant chose in its setup.
- A long run needs `agent.max_turns` raised in Hermes's own `config.yaml`. Hermes ends a turn after that many agent steps (default 90) and waits at the prompt; in the 120-minute practice cells of 22 Oct it stopped after about 7 minutes (measured). For a 168 h run, set it before the start in Goal Cup's Hermes home, for example `HERMES_HOME=<config dir>/capture-homes/hermes-cli hermes config set agent.max_turns 2000`. It is the participant's own setting (D61): Goal Cup Proxy never sets it, and after the run starts the participant may only type "keep going".
- Since D61 the home is not seeded, so Hermes's defaults apply: title generation (an auxiliary model call), compression, the `tirith` command scanner (downloaded from GitHub at start; with the network refused it prints `tirith security scanner enabled but not available`, measured) and the remote model catalogue. The connector's `configure` commands set only how the session is stored: `sessions.write_json_snapshots: true` and `compression.in_place: false` (D61).
- The model host is the one `config.yaml` names in `model.base_url` (D62, `capture.model_host_from`), read at each start and continue; there is no provider list. A switch with `/model` inside a running session is not recorded until the participant exits Hermes and runs `goalcup continue`: until then `HERMES_CA_BUNDLE` (replace mode) makes the new service's calls fail their TLS handshake through the opaque tunnel (inference from source, not measured). A custom `http://` or loopback endpoint is refused at start.
- Known gap, an override of the base URL (source, not measured): for a built-in API-key provider Hermes takes its runtime base URL from `<PROVIDER>_BASE_URL` (for example `DEEPSEEK_BASE_URL` in `$HERMES_HOME/.env`) or from its own registry, not from `config.yaml` (`hermes_cli/runtime_provider.py:1474-1484`). Setup writes the same value to `model.base_url`, so the common case matches (measured). A participant who sets such a variable to another host gets model calls that tunnel and fail their TLS handshake under the replace-mode CA, with no refusal at start.
- Known gap, OAuth-login providers (traced from source, not measured; to be run in the test round): Nous Portal refreshes its token through `hermes_cli/auth.py:5780` `resolve_nous_runtime_credentials`, whose TLS context `_resolve_verify` (`auth.py:4593`) is built from `HERMES_CA_BUNDLE`, which holds the recording proxy CA alone. The refresh goes to the login host (`portal.nousresearch.com`), not the approved `model.base_url` host, so it tunnels with its real certificate and fails its handshake. Model calls then fail with an auth error once the token minted at prep has expired, and the start does not refuse. Codex-OAuth and xAI-OAuth refreshes follow the same pattern. So API-key services work, and OAuth-login services may not, until a test-round run measures it.
- The first reference row passed capture integrity but its trace has not been re-decoded; the claim stays capture-only ([reference row, D41](../DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules)).
- Open: every "unknown" above; the Linux and Windows `HERMES_HOME`; the update-check host; failure exit codes; whether the console script bytes are portable; whether `hermes profile use` inside the shared home redirects runs (the `active_profile` file is not guarded).

## Hermes 0.22: the gateway and certificate trust

Hermes 0.22 (measured with v0.22.5 on macOS, 26 Oct 2026, staging practice runs; source read at
the v0.22.5 checkout) changed two things the 0.19.0 recipe relied on. The connector's
`[launch_prep]` (D72) handles both.

- **The gateway.** `hermes chat` is a thin client: every model call runs in a gateway process,
  one per `HERMES_HOME`, which keeps the environment of whoever started it. A gateway left running
  from `goalcup setup`, or from an earlier run, has no recording proxy in its environment, so its
  calls go straight to the model service and nothing is recorded. With no gateway running,
  `hermes chat` inside the run starts a fresh one with the run's environment (measured: born 1 s
  after the start). So `[launch_prep]` ends with `hermes gateway stop -y`, last, nearest the
  launch. It exits 0 when no gateway runs (measured). Hermes 0.19.0 has no `-y` there and exits 2;
  goalcup logs that line in `host.log` and goes on.
- **Trust.** Hermes 0.22 on macOS ignores `HERMES_CA_BUNDLE`. What it trusts for an endpoint is a
  provider entry's `ssl_ca_cert`, matched by the endpoint's base URL (`agent/ssl_verify.py`
  `resolve_httpx_verify`; entries from `providers:` and `custom_providers:`). That file replaces
  the system roots for that endpoint, so a missing file fails closed. So before each launch
  goalcup writes the run's proxy CA to `recorder-ca.pem` in the home (mode 0600) and runs
  `hermes config set providers.recorder.base_url <the model.base_url goalcup read>` and
  `hermes config set providers.recorder.ssl_ca_cert <that file>`. Both exit 0 and start no
  gateway (measured). The entry covers the main chat client and the auxiliary client (titles,
  compression). It does not change which provider or model Hermes uses: it has no key, model or
  `api_mode`, and is picked only by its name. It does show in the `/model` picker and in
  `hermes doctor`; picking it gives a provider with no key.
- **Result.** With no old gateway and the entry in place, a staging practice run recorded 1.2 MB
  of DeepSeek calls with no certificate error (measured, run 3).
- **Session file.** Hermes 0.21.1 and later write no `session_<id>.json` (section 4). goalcup
  records their model calls only, as for Amp. The connector's native row has
  `warn_missing = false`, so `goalcup status` does not print the "no session file" line for them.
  Old Hermes still writes and records its file. `HERMES_CA_BUNDLE` and the
  `sessions.write_json_snapshots` configure command stay for Hermes up to 0.21.0.
- **`goalcup continue`** opens a new Hermes session on 0.21.1 and later: with no session file it
  cannot ask for `--continue`. The earlier conversation stays in Hermes's own `state.db`.

Known limits (each shows up as the "no model calls recorded" line in `goalcup status`, none is
silent):

- A provider entry of your own for the same URL is matched first, so the recording CA is not
  used and the calls fail.
- A bare `provider: custom` setup may treat the `recorder` entry as the URL's owner (source,
  `hermes_cli/runtime_provider_custom.py`; not verified).
- Hermes's native Anthropic client ignores `ssl_ca_cert` (`agent/anthropic_adapter.py`), so Hermes
  with the Anthropic provider records nothing on 0.21.6 and later. Use another provider, or Hermes
  0.21.0 or older.
- A model URL with a query string (Azure's `?api-version=`) does not match the entry, which holds
  the host, port and path only.
- A gateway another Hermes process starts between the stop and the launch (a kanban or cron
  worker, or a launchd unit from `hermes gateway install`) keeps its old environment
  ([CHEAT-CHECKLIST](../CHEAT-CHECKLIST.md)).

## Run without permission prompts

During a run you may type only "keep going" ([RULES](../RULES.md)), so a harness that stops to ask
waits. Hermes 0.19.0 asks only before a shell command it deems dangerous: with the default
`approvals.mode: smart`, a helper model approves the low-risk ones and the rest get a prompt with a
60 s timeout (`hermes_cli/config.py:2667-2678`, source, not measured). Goal Cup recommends turning
the prompt off and never sets it for you (D67). It goes in `config.yaml` in Hermes's Goal Cup home
(`HERMES_HOME`, under `capture-homes/hermes-cli/` in your Goal Cup configuration folder;
`goalcup setup` prints the full path when it is missing):

```yaml
approvals:
  mode: "off"
```

- `approvals.mode` accepts `manual`, `smart` and `off` (`tools/approval.py:2436`). Quote `"off"`:
  an unquoted `off` is YAML for false, which Hermes also takes as off (`tools/approval.py:2429-2438`).
- `hermes chat --yolo` (or `HERMES_YOLO_MODE=1`, or `/yolo` in a session) does the same for one
  session; the connector passes no such option. Hermes's own deny rules and blocklist still apply
  (`tools/approval.py:338`).
- File edits are not gated (only memory and skill writes are, `tools/write_approval.py`).
- What we saw: the Hermes practice runs of the local test day (lx-hermes, mac-hermes) made their
  commits with no approval stall.
- `goalcup setup` reads `approvals.mode` and, while it is not off, adds the recommendation to its
  harness home step. It never changes it and never fails for it.
- Evidence: [local test day](../research/results/2026-10-22-local-test-day.md),
  [plan v4](../research/results/2026-10-22-fix-plan-v4.md).
- Every setting for a long run, `agent.max_turns` included, for participants:
  [Set up your harness for a 168-hour run](../HARNESS-SETUP.md#hermes).
