# Hermes Agent harness facts

Connector id: `hermes-cli` · Facts for version: `0.22.5` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Nous Research's public Hermes docs and source at tag `v0.22.5`, the project's release page, and Goal Cup runs on macOS in October 2026. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/hermes-cli.toml`](../../connectors/hermes-cli.toml))

Source words: "docs" is [hermes-agent.nousresearch.com/docs](https://hermes-agent.nousresearch.com/docs/user-guide/configuration) (configuration and reference/cli-commands pages) as read on 2026-10-31; "source" is the public repository at `v0.22.5`; "measured" is a Goal Cup run, on 0.19.0 or 0.22.5 as each line says; "unknown" is not measured or read. Releases are git tags; PyPI stops at 0.19.0.

## 1. Run one prompt, in chat mode first

- `hermes chat` opens the classic terminal chat (a bare `hermes` does the same). The Ink screen needs `--tui`, `HERMES_TUI=1` or `display.interface: tui`; the connector passes none ([docs](https://hermes-agent.nousresearch.com/docs/reference/cli-commands)). The participant types at the prompt; `/exit`, `/quit` or Ctrl-D at an empty prompt quits with exit 0 (measured).
- A home with no provider prints "It looks like Hermes isn't configured yet" and asks `Run setup now? [Y/n]`. The wizard picks the provider and model and then Hermes closes without opening a chat (source). The connector's `first_screen` line tells the participant to answer it before typing a prompt.
- `hermes chat --continue` (`-c`) resumes the most recent session (docs). With no earlier session it prints "No previous CLI session found to continue." and exits 1 (measured). The connector's `resume` is `["--continue"]`.
- `hermes chat --oneshot -q "<prompt>"` is the one-shot form and is not used. Hermes `--safe-mode`, `HERMES_SAFE_MODE=1` and `--ignore-user-config` switch capture off ([RULES](../RULES.md)). Hermes reads `-p`/`--profile` from the raw arguments before parsing and switches `HERMES_HOME` (source).

## 2. Install and record a version

- `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash` clones the repository and checks out the latest stable tag; `--skip-setup` makes it non-interactive and `--commit <sha>` pins a build (source, `scripts/install.sh`). The Python package `hermes-agent` on PyPI is 0.19.0 and is behind the tags. Python 3.11 or later; the installer uses 3.14.
- `hermes --version` prints `Hermes Agent v<version> (<date>)` (measured on 0.19.0). The connector sets no `version` word, so any build runs and the `start` event records the one that ran.
- The core does not update itself. A background check runs at chat start and `updates.check: false` in `config.yaml` stops it; there is no environment switch (source). The connector's `update_off` row writes that key into the isolated home. Never run `hermes update` during a run.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HERMES_HOME`, default `~/.hermes`: `config.yaml`, `.env`, `auth.json`, `.anthropic_oauth.json` ([docs](https://hermes-agent.nousresearch.com/docs/user-guide/configuration); measured) | same (docs; Docker uses `/opt/data`) | unknown |
| Session store | `$HERMES_HOME/state.db`, SQLite in WAL mode, with `-wal` and `-shm`; every session ever run in the home is in it (docs; source) | same | unknown |
| Subagent logs | `cache/delegation/live/<id>/task-<n>.log` and `manifest.json`, pruned after 7 days, redacted by Hermes before writing (source) | same | unknown |
| Logs / cache | `logs/agent.log`, rotated at 5 MiB times 3 (docs) | same | unknown |
| Env vars that move these | `HERMES_HOME` moves all of the above (measured); `-p`/`--profile` switches it (source) | same | unknown |

The connector sets `HERMES_HOME` to the isolated home and leaves `HOME` alone.

## 4. Session format and identity

- `state.db` has `sessions`, `messages`, usage and `async_delegations` tables (schema 32 at v0.22.5, source). Hermes 0.21.1 and later write no `session_<id>.json` snapshot (source), so a run on 0.22.5 has no per-session file; its model calls are recorded on the wire. The connector copies `state.db*` as a set when the run ends (`final`).
- A `delegate_task` child is a row with `parent_session_id` set, in the same database, and also has a log under `cache/delegation/live/`. The connector reads `task-*.log` as it grows and copies each `manifest.json` as a snapshot; both may match nothing when no subagent runs, which is not a warning.
- The session id is `YYYYMMDD_HHMMSS_<six hex>` (source). Hermes prints it in the banner; the connector does not read the terminal.

## 5. Auth

- About 40 providers. A key goes in `$HERMES_HOME/.env` (mode 0600; secrets only); the provider and model go in `config.yaml` as `model.provider`, `model.default` and `model.base_url` (docs). OAuth providers keep tokens in `auth.json`; Hermes can also borrow credentials from `~/.codex/auth.json` and `~/.claude/.credentials.json` (source). The connector lists `.env`, `auth.json`, `.anthropic_oauth.json` and `config.yaml` as secret files.
- Does the harness have a variable that moves its config and login? Yes, `HERMES_HOME`; the connector sets it. The participant's setup wizard writes into that home once, in `goalcup setup`.
- A credential-pool entry in `auth.json` keeps `secret_fingerprint`, the first 16 hex digits of the key's SHA-256, and not the key (source). After a failed request, a `request_dump_*.json` in `sessions/` carries the first 8 characters of the key (measured on 0.19.0).

## 6. Network

- Proxy: `HTTPS_PROXY` and `NO_PROXY` are honoured by the HTTP client for model calls; a malformed proxy URL stops the turn with an error. Model calls are HTTP/1.1 and Chat Completions, with no WebSocket (measured).
- CA: since 0.21.6 trust is the OS store through the `truststore` package, patched into every TLS context; `HERMES_CA_BUNDLE` no longer reaches model calls (source). On Linux that package reads `SSL_CERT_FILE` as a replacement for the default file, so goalcup sets it to a bundle of the system roots plus the run CA. Not measured on 0.22.5.
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
- Open: whether Hermes's Anthropic, Gemini-native, Bedrock and Vertex adapters use the process-wide trust setting; delegation, `SSL_CERT_FILE` with a full bundle, and `hermes chat --continue` after a home switch, all on 0.22.5; the Linux and Windows paths; whether `state.db` message rows are redacted; failure exit codes.

## Run without permission prompts

With the default `approvals.mode: smart`, a helper model approves low-risk shell commands and the rest get a prompt with a 60 s timeout (source). The connector's `no_prompts` row writes `approvals.mode = "off"` into `config.yaml` in the isolated home. The mode accepts `manual`, `smart` and `off`; quote `"off"`, because an unquoted `off` is YAML for false, which Hermes also reads as off (source). `hermes chat --yolo` does the same for one session ([docs](https://hermes-agent.nousresearch.com/docs/reference/cli-commands)); the connector passes no such option. File edits are not gated. See [HARNESS-SETUP](../HARNESS-SETUP.md#hermes).

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and [`PAD-RESULTS.md`](../plans/2026-10-30-v1.0/PAD-RESULTS.md); this sheet does not copy them.
