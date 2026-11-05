# Hermes Agent harness facts

Connector id: `hermes-cli` · Facts for version: `0.22.5` to `0.22.7` · Written by: Goal Cup team · Last checked: 2026-11-01
Public disclosure: yes. Facts come from Nous Research's public Hermes docs and source at tag `v0.22.5`, the project's release page, and Goal Cup runs on macOS in October 2026. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/hermes-cli.toml`](../../connectors/hermes-cli.toml))
Not recorded: Hermes session database: not copied (search index).

Source words: "docs" is [hermes-agent.nousresearch.com/docs](https://hermes-agent.nousresearch.com/docs/user-guide/configuration) (configuration and reference/cli-commands pages) as read on 2026-10-31; "source" is the public repository at `v0.22.5`; "measured" is a Goal Cup run, on 0.19.0 or 0.22.5 as each line says; "unknown" is not measured or read. Releases are git tags; PyPI stops at 0.19.0.

## 1. Run one prompt, in chat mode first

- `hermes chat` opens the classic terminal chat (a bare `hermes` does the same). The Ink screen needs `--tui`, `HERMES_TUI=1` or `display.interface: tui`; the connector passes none ([docs](https://hermes-agent.nousresearch.com/docs/reference/cli-commands)). The participant types at the prompt; `/exit`, `/quit` or Ctrl-D at an empty prompt quits with exit 0 (measured).
- A home with no provider prints "It looks like Hermes isn't configured yet" and asks `Run setup now? [Y/n]` The trust entries above do not count as setup: the check reads the `model` section and keys, not `providers` (source). The wizard picks the provider and model and then Hermes closes without opening a chat (source). The connector's `first_screen` line tells the participant to answer it before typing a prompt.
- `hermes chat --continue` (`-c`) resumes the most recent session (docs). With no earlier session it prints "No previous CLI session found to continue." and exits 1 (measured). The connector's `resume` is `["--continue"]`.
- `hermes chat --oneshot -q "<prompt>"` is the one-shot form and is not used. Hermes `--safe-mode`, `HERMES_SAFE_MODE=1` and `--ignore-user-config` switch capture off ([RULES](../RULES.md)). Hermes reads `-p`/`--profile` from the raw arguments before parsing and switches `HERMES_HOME` (source).

## 2. Install and record a version

- `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash` clones the repository and checks out the latest stable tag; `--skip-setup` makes it non-interactive and `--commit <sha>` pins a build (source, `scripts/install.sh`). The Python package `hermes-agent` on PyPI is 0.19.0 and is behind the tags. Python 3.11 or later; the installer uses 3.14.
- `hermes --version` prints `Hermes Agent v<version> (<date>)` (measured on 0.19.0). The version probe runs in the isolated home like the launch, because a probe in the participant's real home would write there (measured on Hermes 0.22.5: `--version` creates `__pycache__` files and touches lease files under `~/.hermes`). In a fresh isolated home 0.22.5 exits 1 with "no dependency environment is committed for this install; run hermes pm repair", so the version is not read: the run starts, says so, and the upload says `unknown` (`approve.rs` `wire_version`; the sent string is cut at 64 UTF-16 units). The connector has no version rows, so any build runs the same way and the `start` event records the one that ran.
- The core does not update itself. A background check runs at chat start and `updates.check: false` in `config.yaml` stops it; there is no environment switch (source). The connector's `update_off` row writes that key into the isolated home. Never run `hermes update` during a run.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HERMES_HOME`, default `~/.hermes`: `config.yaml`, `.env`, `auth.json`, `.anthropic_oauth.json` ([docs](https://hermes-agent.nousresearch.com/docs/user-guide/configuration); measured) | same (docs; Docker uses `/opt/data`) | unknown |
| Session store | `$HERMES_HOME/state.db`, SQLite in WAL mode, with `-wal` and `-shm`; every session ever run in the home is in it (docs; source). Not copied (section 4) | same | unknown |
| Subagent logs | `cache/delegation/live/<id>/task-<n>.log` and `manifest.json`, pruned after 7 days, redacted by Hermes before writing (source) | same | unknown |
| Logs / cache | `logs/agent.log`, rotated at 5 MiB times 3 (docs) | same | unknown |
| Env vars that move these | `HERMES_HOME` moves all of the above (measured); `-p`/`--profile` switches it (source) | same | unknown |

The connector sets `HERMES_HOME` to the isolated home and leaves `HOME` alone.

## 4. Session format and identity

- `state.db` has `sessions`, `messages`, usage and `async_delegations` tables (schema 32 at v0.22.5, source). Hermes 0.21.1 and later write no `session_<id>.json` snapshot (source), so a run on 0.22.5 has no per-session file; its model calls are recorded on the wire. The connector does not copy `state.db`. Hermes keeps its full-text search index (`messages_fts` and `messages_fts_trigram`, each with `_data`, `_idx`, `_docsize` and `_config` shadow tables and sync triggers) inside that one file (source: `hermes_state.py`), so terms stored out of context cannot be masked: measured in full4, a planted password sat raw in `state.db` and in its WAL. There is no setting that turns the index off and no separate index file (measured at v0.22.5 in a keyless box: `SessionDB()` in an empty `HERMES_HOME`; the only toggle found, `cjk_fts`, adds a third index), and the native capture copies files whole.
- Kept: every model call on the wire, the delegation `manifest.json` snapshots and `task-*.log` files. Not in the record: the `sessions`, `messages`, usage and `async_delegations` tables, so Hermes's own per-session totals and `parent_session_id` links. Hermes subagents are shown by their delegation task logs (`cache/delegation/live` `task-*.log` and `manifest.json`), not tied to individual calls, and per-thread usage is not measured (PRD-08 reads them).
- A `delegate_task` child is a row with `parent_session_id` set in `state.db` (not copied), and also has a log under `cache/delegation/live/`. The connector reads `task-*.log` as it grows and copies each `manifest.json` as a snapshot; both may match nothing when no subagent runs, which is not a warning.
- The session id is `YYYYMMDD_HHMMSS_<six hex>` (source). Hermes prints it in the banner; the connector does not read the terminal.

## 5. Auth

- About 40 providers. A key goes in `$HERMES_HOME/.env` (mode 0600; secrets only); the provider and model go in `config.yaml` as `model.provider`, `model.default` and `model.base_url` (docs). OAuth providers keep tokens in `auth.json`; Hermes can also borrow credentials from `~/.codex/auth.json` and `~/.claude/.credentials.json` (source). Goal Cup supplies, stores and uploads no key. Hermes starts with your own shell environment (proxy variables replaced) and its own home files, so it finds your key where it always does. The home files are the connector's isolated `HERMES_HOME`, which the setup wizard fills once in `goalcup setup`, so a key kept only in your normal `~/.hermes` is not seen until setup writes it there. Goal Cup reads only base-URL settings from `config.yaml` and `.env` to name hosts.
- Does the harness have a variable that moves its config and login? Yes, `HERMES_HOME`; the connector sets it. The participant's setup wizard writes into that home once, in `goalcup setup`.
- The first screen tells the participant: "Your key is found where Hermes always finds it, and Goal Cup stores and uploads no key." and "Subagents are shown by their task logs, not tied to single calls, and per-thread usage is not measured."
- A credential-pool entry in `auth.json` keeps `secret_fingerprint`, the first 16 hex digits of the key's SHA-256, and not the key (source). After a failed request, a `request_dump_*.json` in `sessions/` carries the first 8 characters of the key (measured on 0.19.0).

## 6. Network

- Proxy: `HTTPS_PROXY` and `NO_PROXY` are honoured by the HTTP client for model calls; a malformed proxy URL stops the turn with an error. Model calls are HTTP/1.1 and Chat Completions, with no WebSocket (measured).
- CA: since 0.21.6 (unchanged through 0.22.7) model calls trust the OS store through the `truststore` package plus the `ssl_ca_cert` of the first `custom_providers` or `providers` entry whose `base_url` is the route (`agent/ssl_verify.py:300-343`, `hermes_cli/config_providers.py:446-472`; source). On Linux that package reads `SSL_CERT_FILE`, which goalcup sets to the system roots plus the run CA; on macOS it never reaches model calls, on every version (0.22.5, 0.22.6 and 0.22.7 alike; no pad-only config is needed). So at every start and continue goalcup's `ca_settings` give each address in `config.yaml` (`model.base_url`, `delegation.base_url`, `auxiliary.*.base_url`, every `custom_providers` and `providers` entry) `ssl_ca_cert: ${env:SSL_CERT_FILE}`: on the participant's own entry when one holds the address (an entry with its own CA keeps it: it points at a file in goalcup's Hermes home holding that CA, then the run bundle, and the participant's file is never changed; when their CA file cannot be read the entry is left as it is and the start says calls to that address may fail on macOS), else on a `goalcup-trust-<address>` entry under `providers`. The addresses are read from the participant's own `config.yaml`, built-in provider or custom `base_url`, at each start and continue. Only when it names none (a fresh home, or a provider picked without its URL) are the entries for Hermes's nine built-in routes (DeepSeek, Vercel AI Gateway, OpenRouter, Nous, xAI, Z.ai, Fireworks, Alibaba and Alibaba international). An entry names only the real address, never the proxy: Hermes still calls the model itself through `HTTPS_PROXY`. The entries show in Hermes's provider list and are not for picking. Only DeepSeek and the Vercel AI Gateway are measured; other routes are not verified until a pad row proves them.
- The gateway: `hermes chat` is a thin client; every model call runs in a gateway process, one per `HERMES_HOME`, which keeps the environment of whoever started it (source; measured). A gateway already running has no recording proxy in its environment and calls the model directly. The connector's `before` and `after` rows run `hermes gateway stop -y` ([docs](https://hermes-agent.nousresearch.com/docs/reference/cli-commands)): with no gateway running it exits 0 (measured on 0.22.5), and `hermes chat` then starts a fresh gateway with the run's environment. Hermes 0.19.0 has no `-y` there and exits 2 (measured).
- Hosts: whatever `model.base_url` or the provider's own address names; the connector hints `config.yaml` (`model.base_url`, `providers.*`, `custom_providers.*`, `delegation.base_url`, `auxiliary.*`), the `*_BASE_URL` keys in `.env`, and the `*_BASE_URL` names in the connector's `hint_env`. `models.dev`, `api.github.com`, `github.com` and `portal.nousresearch.com` are asked for metadata, tools and sign-in and pass as tunnels. No telemetry by default (docs).

## 7. Process model

- A Python process; tool processes start in their own sessions (source). Subagents run inside the gateway process, up to 10 at once, depth 1; interactive delegations run in the background and report between turns (source). Delegation on 0.22.5 was not measured; it failed on 0.19.0 on 29 Oct with an internal executor error and worked on a Mac on 22 Oct.
- `copilot-acp` runs GitHub's Copilot CLI as a child with its own stack. Its model calls go through the proxy, but its session files go to its own home, which no connector row reads: wire only ([RULES](../RULES.md)).
- Exit codes: 0 at `/exit`, 1 at the setup question's Ctrl-C and with no earlier session under `--continue` (measured); others unknown.

## 8. Things Goal Cup cannot isolate

- A gateway another Hermes process starts between the stop and the launch (a cron or kanban worker, or a service from `hermes gateway install`) keeps its own environment and calls direct.
- Host credentials: Hermes reads credential files of other tools from `~/.codex` and `~/.claude` (source).
- A second `HERMES_HOME`, a `--profile`, a Docker container or the desktop app pointed at a remote `hermes serve` is invisible to goalcup.
- Machine-wide managed settings: unknown.

## 9. Quirks and open questions

- `agent.max_turns` defaults to 90 steps; Hermes then waits at the prompt. Raise it in `config.yaml` before a long run ([HARNESS-SETUP](../HARNESS-SETUP.md#hermes)).
- The first start downloads a command scanner from GitHub unless `security.tirith_enabled: false`; with the network refused it prints a warning (measured on 0.19.0).
- Open: whether Hermes's Anthropic, Gemini-native, Bedrock and Vertex adapters use the process-wide trust setting; delegation, `SSL_CERT_FILE` with a full bundle, and `hermes chat --continue` after a home switch, all on 0.22.5; the Linux and Windows paths; whether delegation task logs hold text outside masking shapes: not measured; failure exit codes.

## Run without permission prompts

With the default `approvals.mode: smart`, a helper model approves low-risk shell commands and the rest get a prompt with a 60 s timeout (source). The connector's `no_prompts` row writes `approvals.mode = "off"` into `config.yaml` in the isolated home. The mode accepts `manual`, `smart` and `off`; quote `"off"`, because an unquoted `off` is YAML for false, which Hermes also reads as off (source). `hermes chat --yolo` does the same for one session ([docs](https://hermes-agent.nousresearch.com/docs/reference/cli-commands)); the connector passes no such option. File edits are not gated. See [HARNESS-SETUP](../HARNESS-SETUP.md#hermes).

Telemetry screen: on a home that never answered, an interactive chat first shows "Help improve Hermes?" and waits behind it (rung 1, 2026-10-31: 0 requests until seal). The screen is skipped when `telemetry.shared_metrics.enabled` is true, or when a key is present and `offer_version` is 2 (source: `hermes_cli/observability/shared_metrics_consent.py`, `cli-config.yaml.example`, v0.22.5). A settings row cannot write the integer `offer_version`. Writing `enabled = true` would answer the screen for the participant and turn Hermes's usage counting on; `enabled` turns collection on and `send` turns the daily upload on, and Hermes's own offer text says data from before opt-in is never sent apart from a one-time install note, a note the source does not tie to a key. The setup question asks about approvals only, so the connector writes no telemetry row and the `first_screen` line tells the participant to pick "No thanks".

First run of a new `HERMES_HOME` installs about 105 Python packages (72 MB from files.pythonhosted.org). Measured in rung 1.

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `scripts/pad/rows/PAD-RESULTS.md`; this sheet does not copy them.
