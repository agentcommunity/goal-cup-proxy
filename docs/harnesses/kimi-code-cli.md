# Kimi Code harness facts

Connector id: `kimi-code-cli` · Facts for version: `2.4.1` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Moonshot's public docs and the `2.4.1` npm package run in a build box. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/kimi-code-cli.toml`](../../connectors/kimi-code-cli.toml))

Source words: "docs" is [moonshotai.github.io/kimi-code](https://moonshotai.github.io/kimi-code/en/configuration/env-vars) as read on 2026-10-31; "package" is the `2.4.1` source bundle; "unknown" is not read or measured. `2.4.1` was the current npm release on 30 Oct 2026. Nothing was run through the proxy: the pad has not seen it.

## 1. Run one prompt, in chat mode first

- `kimi` with no argument opens the TUI. `kimi --continue` (`-c`) resumes the most recent session in the current folder ([docs](https://moonshotai.github.io/kimi-code/en/reference/kimi-command)). The connector's `resume` is `["--continue"]`.
- `kimi -p "<prompt>"` runs once and prints; it always uses auto permission and cannot be combined with `--yolo` (docs). It is not chat mode and is not used.
- With no stored sign-in, Kimi asks to sign in. The connector's `first_screen` line says to do it before typing a prompt. How to quit: not on the docs page read.

## 2. Install and record a version

- `npm install -g @moonshot-ai/kimi-code@<version>` (Node 22.19 or later) or `curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash` ([docs](https://moonshotai.github.io/kimi-code/en/guides/migration)).
- `kimi --version` (`-V`) prints the version (docs). The connector sets no `version` word.
- `KIMI_CODE_NO_AUTO_UPDATE=1` "fully disable[s] the update preflight: no check, background install, or prompt" ([docs](https://moonshotai.github.io/kimi-code/en/configuration/env-vars)). The connector sets it.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HOME/.kimi-code/` (`config.toml`, `tui.toml`, `mcp.json`) ([docs](https://moonshotai.github.io/kimi-code/en/configuration/data-locations)) | same | `%USERPROFILE%\.kimi-code\` |
| Session store | `sessions/<workspaceId>/<sessionId>/`; `session_index.jsonl` beside `sessions/`, not inside | same | same |
| Logs / cache | `logs/kimi-code.log`, `bin/`, `updates/`, `user-history/` | same | same |
| Env vars that move these | `KIMI_CODE_HOME` | same | same |

`KIMI_CODE_HOME` moves config, sessions, logs, OAuth credentials and Kimi-specific skills. Generic `.agents` skills and `AGENTS.md` stay under the real home (docs). `HOME` stays the participant's, so goalcup writes no `.profile` for Kimi.

## 4. Session format and identity

- JSONL, one `wire.jsonl` per agent: `agents/main/wire.jsonl` for the main agent and `agents/agent-N/wire.jsonl` for each subagent; `state.json` holds title, last prompt and timestamps (docs). The connector reads `session_index.jsonl` at the home root as row `index` and every `wire.jsonl` under `sessions/` as row `wire`.
- The workspace id is `wd_<slug>_<12 hex>` or `wm_<uuid>`; the session id is the folder name (docs).

## 5. Auth

- Kimi OAuth sign-in, stored as `credentials/<name>.json`, mode 0600 (docs), or a provider key in `config.toml`.
- Provider config: `[providers.<name>]` with `type` (`kimi`, `anthropic`, `openai`, `openai_responses`, `google-genai`, `vertexai`), `base_url`, and exactly one of `api_key` or `api_key_env` ([docs](https://moonshotai.github.io/kimi-code/en/configuration/providers)). DeepSeek uses `type = "openai"`; the docs have no DeepSeek example.
- Rule: shell variables such as `OPENAI_API_KEY` are not read as credentials. Only a declared `api_key_env`, or the `KIMI_MODEL_*` family, reads the shell (docs). Use `api_key_env`, so no key is written to `config.toml`.
- A variable that moves config and login together: yes, `KIMI_CODE_HOME`.

## 6. Network

- Proxy: `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`; loopback always bypasses the proxy (docs). Node bundle: it installs an undici environment proxy agent and passes `NODE_USE_ENV_PROXY=1` and the proxy variables to child processes (package).
- CA: `NODE_EXTRA_CA_CERTS` is the Node standard; the docs list no CA variable and the package does not mention one. Unverified, so the pad's first row is its proof. The connector has no `ca_env`; goalcup sets `NODE_EXTRA_CA_CERTS` to the run CA for every harness.
- Where the model host is named: `providers.*.base_url` and `providers.*.env.*_BASE_URL` in `config.toml`. Hint variables: `KIMI_MODEL_BASE_URL` (only with `KIMI_MODEL_NAME` set), `KIMI_CODE_BASE_URL`.
- `api.kimi.com`, `api.kimi.ai`, `api.moonshot.ai` and `api.moonshot.cn` are listed provider hosts. Sign-in hosts `auth.kimi.com` and `auth.kimi.ai` are on the never-decrypt list and pass through as tunnels.
- Not found: HTTP/2, a WebSocket model path, pinning.

## 7. Process model

- One Node process. Subagents (`Agent`, `AgentSwarm`; built-in `coder`, `explore`, `plan`) run inside the same session state machine, so in process (package; unverified). Background tasks keep status under `tasks/`.
- Exit codes and SIGTERM: unknown.

## 8. Things Goal Cup cannot isolate

- `~/.agents/skills/` and `~/.agents/AGENTS.md`, which stay under the real home and are read by the run.
- Plugins and MCP servers the participant adds; their traffic is the participant's choice.

## 9. Quirks and open questions

- The managed Kimi service and a custom provider are different hosts; a run on a DeepSeek provider never calls `api.kimi.com` for models.
- Unknown: quitting, exit codes, whether `--continue` finds the session after a home switch, Windows paths.
- Unknown: whether `NODE_EXTRA_CA_CERTS` is honoured; the first run settles it.

## Run without permission prompts

None set. `--yolo` ("ask when needed") and `--auto` ("never ask") are launch flags, not a settings key the docs name ([docs](https://moonshotai.github.io/kimi-code/en/reference/kimi-command)), so the connector has no `no_prompts` row and no flag. The participant picks the mode in the TUI. See [HARNESS-SETUP](../HARNESS-SETUP.md).

## Pad result

Not yet run on the pad. The tier and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml).
