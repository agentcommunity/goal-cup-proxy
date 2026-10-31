# Goose harness facts

Connector id: `goose-cli` · Facts for version: `1.57.0` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Goose's public docs, the public source at `v1.57.0` and the 1.57.0 release binary run in a build box (path and flag checks only). No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/goose-cli.toml`](../../connectors/goose-cli.toml))

Source words: "docs" is [goose-docs.ai](https://goose-docs.ai/docs/guides/environment-variables) as read on 2026-10-31; "source" is `aaif-goose/goose` at `v1.57.0`, read on 29 Oct 2026 and not run; "box" is the 1.57.0 binary in a build box; "unknown" is not measured or read. No model call was made through goalcup on a real Goose this cycle.

## 1. Run one prompt, in chat mode first

- `goose session` starts the chat; the connector's `args` is `["session"]`. `goose session --resume` resumes the previous session ([docs](https://goose-docs.ai/docs/guides/goose-cli-commands)); the connector's `resume` is `["--resume"]`, and source takes the most recently used one.
- `goose run -t "<prompt>"` is the one-shot form; it is not chat mode and is not used. `--no-session` on either form stores no session file, and then there is nothing for the native row to read (docs).
- A new home asks about usage data or a provider before the prompt. The connector's `first_screen` line tells the participant to answer then. How to quit: unknown.

## 2. Install and record a version

- `download_cli.sh` from the repository's `stable` release, with `GOOSE_VERSION=v1.57.0 CONFIGURE=false` (source); it installs into `$HOME/.local/bin`. About one release a week.
- `goose --version` is expected to print the version (not read on the pages). The connector sets no `version` key.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `~/.config/goose/config.yaml` ([docs](https://goose-docs.ai/docs/guides/config-files)) | same | `%APPDATA%\Block\goose\config\config.yaml` (docs) |
| Session store | see below | `~/.local/share/goose/sessions/sessions.db` ([docs](https://goose-docs.ai/docs/guides/logs)) | `%APPDATA%\Block\goose\data\sessions\sessions.db` (docs) |
| Logs / cache | `~/.local/state/goose/logs/cli/<date>/`, deleted after two weeks (docs) | same | `%APPDATA%\Block\goose\data\logs\cli\` (docs) |
| Env vars that move these | `GOOSE_PATH_ROOT` (docs) | same | same |

The environment-variables page gives `~/Library/Application Support/Block/goose/` as the macOS default root, while the logs page lists the Linux path for "Unix-like" systems: which one a macOS CLI uses is unknown. `GOOSE_PATH_ROOT` moves the config, data and state folders under it and nothing else (box), so the connector's `{home}/config`, `{home}/data` and `{home}/state` hold `config.yaml`, `secrets.yaml`, `custom_providers/`, `sessions/sessions.db` and the logs. The logs folder also takes `llm_request.*.jsonl` files (docs).

## 4. Session format and identity

- One SQLite database for all sessions, with `-wal` and `-shm` files. Subagents are rows of the same tables with `session_type = 'sub_agent'` (source). The connector copies `sessions.db*` as a set when the run ends (`final`). Column names and the session id form: unknown.

## 5. Auth

- Bring a key: from the environment first, then the system keyring, and where there is no keyring, the plain file `config/secrets.yaml`; any value of `GOOSE_DISABLE_KEYRING` forces the file ([docs](https://goose-docs.ai/docs/guides/environment-variables)). The connector lists `{home}/config/secrets.yaml` as a secret file.
- Does the harness have a variable that moves its config and login? Yes, `GOOSE_PATH_ROOT`; the connector sets it. The system keyring is not under it (section 8).

## 6. Network

- Proxy: `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` are documented ([docs](https://goose-docs.ai/docs/guides/environment-variables)); loopback targets skip the proxy (source). Goose is a Rust binary on rustls: no Node, no `NODE_USE_ENV_PROXY` need.
- CA: `GOOSE_CA_CERT_PATH` adds the run CA to every provider client (source; not on the environment-variables page read). `SSL_CERT_FILE` is read in replace mode, so goalcup sets it to the full bundle, run CA and system roots.
- Hosts: whatever provider the participant picks, decrypted when it is on the shared list. DeepSeek goes through a custom provider file in `config/custom_providers/*.json` with `engine`, `base_url` and `api_key_env` ([docs](https://goose-docs.ai/docs/getting-started/providers)); the connector hints its `base_url`.
- Model address hints: the keys `*_HOST`, `*_ENDPOINT` and `ASTRON_BASE_URL` in `config.yaml`, and the names in the connector's `hint_env`. The providers page names `ATOMIC_CHAT_HOST` in a note, not in its table. A name Goose adds later is one line in the connector.
- Telemetry is off by default (`GOOSE_TELEMETRY_ENABLED`, docs). No pinning found (source).

## 7. Process model

- Subagents come from the built-in summon extension and are tokio tasks in the same process (source), so they share the proxy and CA. Shell tools are child processes that inherit the environment. Goose limits five running at once per parent (`GOOSE_MAX_CONCURRENT_SUBAGENTS`, docs) and gives each a default timeout of five minutes (Goose docs, `subagents`); these are Goose's limits, not ours. Subagents work only in `GOOSE_MODE=auto`, the default (docs).
- Exit codes and SIGTERM: unknown.

## 8. Things Goal Cup cannot isolate

- A host variable without a scheme is skipped. `OLLAMA_HOST` defaults to `localhost:11434` and `OPENAI_HOST` to `api.openai.com`, neither with `https://`; goalcup's hint check reads URLs only and logs "not a URL, skipped" naming the variable. So a schemeless `OLLAMA_HOST` gives no `local_model` line, and a schemeless custom `OPENAI_HOST` stays a tunnel until the participant adds the host. Nothing is lost silently.
- A Databricks or Snowflake host exported in the shell is decrypted and recorded for the run, whoever uses it, including the agent's shell tools and their query traffic. The two names stay in the list because they are Goose's own provider variables; secrets in that traffic are masked like any other. A participant who does not use those providers should not export them for the run.
- The system keyring, if keys are kept there. An MCP extension that opens its own connections, and another CLI run as a Goose extension (a different harness, which [RULES](../RULES.md) does not allow).

## 9. Quirks and open questions

- The CLI does not update itself; `goose update` is manual ([docs](https://goose-docs.ai/docs/guides/updating-goose)). The connector sets no update switch.
- Unknown: the macOS root (section 3), how the session id appears, Windows behaviour of `GOOSE_PATH_ROOT`.

## Run without permission prompts

None needed: `GOOSE_MODE` defaults to `auto`, which runs tools without asking (docs). The connector has no `no_prompts` row.

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and [`PAD-RESULTS.md`](../plans/2026-10-30-v1.0/PAD-RESULTS.md); this sheet does not copy them.
