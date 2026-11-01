# Codex harness facts

Connector id: `codex-cli` · Facts for version: `0.165.0` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from OpenAI's public Codex docs and source at tag `rust-v0.165.0`, the npm registry, and Goal Cup runs on macOS and Linux in September and October 2026. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/codex-cli.toml`](../../connectors/codex-cli.toml))

Source words: "docs" is [learn.chatgpt.com/docs/config-file/config-advanced](https://learn.chatgpt.com/docs/config-file/config-advanced) as read on 2026-10-31; "source" is the public repository at `rust-v0.165.0` (`codex-rs/`); "measured" is a Goal Cup run, on 0.153.0 to 0.156.1, older than the version above; "unknown" is not measured or read. Nothing was run on 0.165.0. Releases come about once a week.

## 1. Run one prompt, in chat mode first

- `codex` with no argument opens the chat. The participant types the prompt; one Ctrl-C at an idle prompt quits and prints `To continue this session, run: codex resume <id>` (measured). The connector's args are `--no-daemon -C <workspace>`, and `resume` is `["resume", "--last"]`. The parser accepts that form (measured); whether it appends to the same rollout file in the isolated home is not measured.
- No trust prompt appeared on 0.153.0 in a fresh folder; the connector's `first_screen` line still tells the participant to answer one if it appears.
- `codex exec "<prompt>"` is the one-shot form; it is not chat mode and is not used. Do not use `--ephemeral`, `--remote ws://`, `queue`, `fork` or `remote-control` ([RULES](../RULES.md)).
- Modal prompts can swallow a typed turn: the update selector, "Approaching rate limits", and model-migration prompts (measured on 0.153.0).

## 2. Install and record a version

- `npm install -g @openai/codex@0.165.0`, a Node wrapper that starts the native Rust binary; or `curl -fsSL https://chatgpt.com/codex/install.sh | sh`; or a Homebrew cask (source, `README.md`). The native binary sits in the platform package, for example `vendor/<arch>-unknown-linux-musl/bin/codex` on Linux (measured on 0.153.0).
- `codex --version` prints `codex-cli <version>` (measured). The connector sets no `version` word, so any build runs and the `start` event records the one that ran.
- Codex installs nothing by itself. The terminal app checks at start and offers an upgrade that runs `npm install -g`; `check_for_update_on_startup = false` in `config.toml` stops the prompt (source; measured on 0.153.0). The connector's `update_off` row writes that key into the isolated home.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$CODEX_HOME`, default `~/.codex`: `config.toml`, `auth.json`, `.env`, `history.jsonl` (docs; measured) | same (docs; a logged-in Linux run was not measured) | unknown |
| Session store | `sessions/YYYY/MM/DD/rollout-<local time>-<thread id>.jsonl`, one per thread, child threads too (source; measured) | same | unknown |
| State database | `$CODEX_HOME/state_*.sqlite` and other SQLite files for logs, goals and thread edges (measured) | same | unknown |
| Logs / cache | inside `$CODEX_HOME`: `cache/`, `plugins/cache/`, `shell_snapshots/`, `thread-writer-locks/`, `tmp/arg0/`, `version.json` while the update check is on (measured) | same; the first start shallow-clones a plugin marketplace into `.tmp/plugins` with `git` (measured, no login) | unknown |
| Env vars that move these | `CODEX_HOME` and `CODEX_SQLITE_HOME`; `HOME` moves user skills (`~/.agents/skills`) and the login shell's startup files (measured) | same | unknown |

The connector sets `CODEX_HOME` and `CODEX_SQLITE_HOME` to the isolated home. Rollouts older than 7 days are compressed to `.jsonl.zst` in the background (docs; not observed).

## 4. Session format and identity

- Rollouts are JSONL, `{timestamp, type, payload}`, append-only, with a gap-free `ordinal` that stayed contiguous across a kill and a resume (measured). The connector reads every `rollout-*.jsonl` under `sessions` as it grows, and copies the SQLite files (`*.sqlite*`) as a set when the run ends. The state files hold the parent and child thread edges, so they are copied whole, whatever their size.
- The thread id is a time-ordered UUID. It is not known at launch: the rollout file appears when the first message is submitted, and its name carries the start time and the id (measured). A typed prompt is a user-role message shaped like the harness's own context messages (measured on 0.153.0).
- A subagent thread is its own rollout file; its first `session_meta` names the parent and its second replays the parent's (measured). A revert keeps the thread id and writes a new file with a `_<rollout id>` suffix, so watch the folder, not one file (docs).
- `codex resume <id>` appends to the same rollout when it can take `thread-writer-locks/<id>.lock`; when it cannot, it silently becomes a new thread (measured).

## 5. Auth

- ChatGPT sign-in: `auth.json` has `auth_mode` `chatgpt` (measured). API key: `printenv OPENAI_API_KEY | codex login --with-api-key` stores it in `auth.json`; `CODEX_API_KEY` works for `exec` only (source). `OPENAI_API_KEY` in the shell passes through to Codex unchanged; goalcup never reads it.
- Does the harness have a variable that moves its config and login? Yes, `CODEX_HOME`; the connector sets it. The connector lists `auth.json` and `.env` as secret files. Copying `auth.json` to a second home and refreshing there invalidated the first token (measured), so one isolated home is shared by all runs.
- Credentials are in `auth.json` on macOS (measured); Linux and Windows: unknown. Token-count events carry the account's plan type and credit balance; the connector's `mask` rules cover the installation id, the safety identifier and the turn-state value.

## 6. Network

- CA: `CODEX_CA_CERTIFICATE`, else `SSL_CERT_FILE`; the file is added to the native roots for HTTPS and WebSocket alike (source). The connector names `CODEX_CA_CERTIFICATE`.
- Proxy: `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY` are read for the HTTPS client and the Responses WebSocket dialer (source). Codex is Rust (rustls), not Node. A loopback forward proxy relayed the WebSocket (measured, 0.153.0).
- `$CODEX_HOME/.env` is loaded at start and overwrites every non-`CODEX_` variable, so `HTTPS_PROXY`, `NO_PROXY` or `SSL_CERT_FILE` there beat goalcup's; the connector refuses to start if the file sets a proxy or CA value (source). A shared app-server daemon under `CODEX_HOME` makes calls with its own environment; `--no-daemon` makes Codex use the embedded server (source).
- Hosts: `api.openai.com`, first as a WebSocket upgrade to `/v1/responses` with `response.create` frames, then as HTTPS and server-sent events if that fails; with ChatGPT sign-in `chatgpt.com` at `/backend-api/codex/responses`. `auth.openai.com` carries sign-in. A key-less run gets 401 on both transports and exits 1 (measured).
- Model address: `OPENAI_BASE_URL` in the environment or in `.env`, `openai_base_url`, and `model_providers.*.base_url` in `config.toml`; the connector hints all of them. `wire_api = "chat"` is no longer supported (source). goalcup never sets any of these.
- Telemetry is opt-in (`[otel]` exporter defaults to none, docs). The update host: unknown.

## 7. Process model

- Three layers: the npm wrapper (`node`), the native `codex` binary and `codex-code-mode-host` in its own process group. Killing the wrapper alone leaves the native binary running and holding the writer lock (measured).
- Subagents (`spawn_agent` and related tools) are threads inside the same process, up to 6 at once, depth 1 (source); each is a separate rollout file and its model calls go to the same hosts.
- Each agent command runs as `/bin/sh -lc`. A login shell on Debian and Ubuntu resets `PATH`, so with a moved `HOME` the participant's `~/.local/bin` was lost (measured, 0.153.0). The connector does not move `HOME`.
- Exit codes: 0 after a completed `exec` turn, 1 with no login (measured); others unknown.

## 8. Things Goal Cup cannot isolate

- Account state on OpenAI's side: plugins, apps, connectors and memory summaries arrive from the account.
- A shared `CODEX_HOME` across runs carries older runs' sessions and the state databases, which the end-of-run copy takes whole.
- Machine-wide managed configuration: path unknown. The connector seeds no `requirements.toml`.
- A second Codex: another `CODEX_HOME`, an older Homebrew copy, or the Codex desktop app, which shares `CODEX_HOME` (docs).
- No sandbox comes from goalcup: Codex runs with its own defaults and the participant's settings.

## 9. Quirks and open questions

- Untrusted hook definitions are skipped silently with exit 0 (measured on 0.153.0).
- Rollout names and date folders use local time (measured).
- Open: a logged-in run on 0.165.0, the WebSocket path through the proxy on 0.165.0 (measured on 0.153.0 only), the Linux login location, and whether a SIGTERM removes the writer lock (two measurements disagree).

## Run without permission prompts

Codex asks for approval whenever its sandbox blocks a command, and a run allows only "keep going". The connector's `no_prompts` rows write two keys into `config.toml` in the isolated home: `approval_policy = "never"` and `sandbox_mode = "danger-full-access"` ([docs](https://learn.chatgpt.com/docs/config-file/config-advanced)). With `never` alone, commands the default `workspace-write` sandbox blocks fail instead of asking, and the agent cannot commit, because `.git` is read-only there (measured). In a Linux container the sandbox cannot start at all (measured). See [HARNESS-SETUP](../HARNESS-SETUP.md#codex).

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `docs/plans/2026-10-30-v1.0/PAD-RESULTS.md`; this sheet does not copy them.
