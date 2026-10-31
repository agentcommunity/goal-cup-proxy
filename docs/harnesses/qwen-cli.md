# Qwen Code harness facts

Connector id: `qwen-cli` · Facts for version: `0.26.4` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Qwen Code's public docs and the public source at `v0.26.4`. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/qwen-cli.toml`](../../connectors/qwen-cli.toml))

Source words: "docs" is [qwenlm.github.io/qwen-code-docs](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/) as read on 2026-10-31; "source" is `QwenLM/qwen-code` at `v0.26.4`, read on 29 Oct 2026 and not run; "unknown" is not measured or read. Nothing was run on a real Qwen Code this cycle, so no fact here is "measured".

## 1. Run one prompt, in chat mode first

- `qwen` with no argument opens the interactive session. `qwen --continue` resumes "the most recent session for this project"; `--resume [sessionId]` resumes one or lets the user choose ([docs](https://qwenlm.github.io/qwen-code-docs/en/users/features/headless/)). The connector's `resume` is `["--continue"]`.
- `qwen -p "<prompt>"` is the one-shot form; it is not chat mode and is not used. How to quit: unknown.

## 2. Install and record a version

- `npm install -g @qwen-code/qwen-code@0.26.4`, Node 22 or later; a standalone installer that bundles its own Node also exists (source, README). Ten stable releases between 29 Sep and 29 Oct (npm), five of them in nine days, so the run may use a newer build than this sheet.
- The connector sets no `version` key; the `start` event records the version that ran.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `~/.qwen/settings.json`; project `.qwen/settings.json` ([docs](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/)) | same | unknown |
| Session store | `~/.qwen/projects/<sanitized-cwd>/chats/<id>.jsonl` ([docs](https://qwenlm.github.io/qwen-code-docs/en/users/features/headless/)) | same | unknown |
| Logs / cache | the runtime folder, with conversations, logs and todos (docs) | same | unknown |
| Env vars that move these | `QWEN_HOME` (the global config folder, default `~/.qwen`) and `QWEN_RUNTIME_DIR` (conversations, logs, todos; defaults to `QWEN_HOME`) (docs) | same | same |

The connector sets both variables to the isolated home, so `settings.json`, `.env` and `projects/` sit directly under `{home}`. System settings sit outside it: `/etc/qwen-code/settings.json` on Linux, `/Library/Application Support/QwenCode/settings.json` on macOS, and they outrank user and project settings (docs).

## 4. Session format and identity

- JSONL, one file per session, `projects/<sanitized-cwd>/chats/<sessionId>.jsonl`. Qwen's own subagents write `projects/<cwd>/subagents/<sessionId>/agent-<id>.jsonl` and `agent-<id>.meta.json` (source). The connector reads every `.jsonl` under `projects` as it grows, and the `.meta.json` files whole when they change.
- How the session id first appears on screen: unknown.

## 5. Auth

- The free Qwen sign-in ended on 2026-04-15 and is no longer in the `/auth` menu; the choices are QwenCloud, Alibaba ModelStudio, third-party providers (DeepSeek is a preset) and a custom provider ([docs](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/)).
- Keys come from the shell, from `.qwen/.env`, `<QWEN_HOME>/.env` or `~/.env` (the first file found is used, files are not merged), or from the `env` block of `settings.json`, "stored in plain text" (docs). The connector lists `{home}/settings.json` and `{home}/.env` as secret files.
- Does the harness have a variable that moves its config and login? Yes, `QWEN_HOME`; the connector sets it.

## 6. Network

- Proxy: the order is `--proxy`, then the `proxy` setting, then `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy` (source; the docs name `--proxy` and the setting). `NO_PROXY` is honoured through a process-wide undici proxy agent (source). So a `proxy` setting beats goalcup's variable: the connector refuses the start, naming file and key and never the value, if `proxy` is set in `{home}/settings.json` or `{workspace}/.qwen/settings.json`. goalcup passes no `--proxy`.
- CA: `NODE_EXTRA_CA_CERTS` adds to the roots ([docs](https://qwenlm.github.io/qwen-code-docs/en/users/support/troubleshooting/)). Node: yes.
- Hosts: `api.deepseek.com` and the `dashscope` hosts are provider hosts; `chat.qwen.ai` (sign-in) is never decrypted. Usage statistics go to an `rum.aliyuncs.com` host and the model catalog is read from `models.dev` (source); both pass as tunnels.
- Model address: providers are in `modelProviders` in `settings.json`, each with a `baseUrl`; the connector hints `modelProviders.*.*.baseUrl` in the user and workspace settings. A key in the environment (`OPENAI_BASE_URL`) is not hinted. goalcup never sets a model address.
- Calls are HTTP/1.1; no WebSocket or gRPC on the model path (source, unverified by a run).

## 7. Process model

- Qwen's own subagents (tool `agent`, default type `general-purpose`), teams and Arena run in the same process (source). Fork subagents cannot spawn more.
- The built-in `claude-code` and `codex` subagents "delegate to separately installed native tools": Claude Code through an ACP executor, Codex through `codex app-server --stdio` (docs, [sub-agents](https://qwenlm.github.io/qwen-code-docs/en/users/features/sub-agents/)). Qwen does not fall back to its own model if the tool is missing.
- Exit codes and SIGTERM: unknown.

## 8. Things Goal Cup cannot isolate

- Wire only: a `claude-code` or `codex` subagent inherits the run environment, so its model calls go through the proxy and are recorded when the host is listed. Its session files go to that tool's own folder in the real home (`~/.claude`, `~/.codex`), which goalcup does not read or touch. Starting another harness this way is not allowed by [RULES](../RULES.md).
- System settings files (section 3) and a `--proxy` flag typed by the participant are not checked.
- `--sandbox` and `qwen serve` (a background mode, opt-in) keep their own environment (source, unverified).

## 9. Quirks and open questions

- `general.enableAutoUpdate` is `true` by default and has no environment switch (docs). The connector's `update_off` row writes `false` into `{home}/settings.json` at every start and continue.
- Unknown: whether the update check runs in `-p` mode, how the subagent transcripts of a `claude-code` or `codex` subagent appear, Windows paths.

## Run without permission prompts

`tools.approvalMode` takes `plan`, `default`, `auto-edit`, `auto` or `yolo`; `yolo` approves every tool call ([docs](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/)). The connector's `no_prompts` row writes `yolo` into `{home}/settings.json`.

## Pad result

Not yet run on the pad. The tier and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml).
