# GitHub Copilot CLI harness facts

Connector id: `copilot-cli` · Facts for version: `1.0.105` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from GitHub's public Copilot CLI docs, its public changelog and strings in the shipped package. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/copilot-cli.toml`](../../connectors/copilot-cli.toml))

Source words: "docs" is [docs.github.com/copilot](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference) as read on 2026-10-31; "changelog" is the project's public changelog; "package" is strings and `--help` of the 1.0.105 package; "unknown" is not measured or read. Nothing was run on a real Copilot CLI this cycle, so no fact here is "measured".

## 1. Run one prompt, in chat mode first

- `copilot` with no argument opens the interactive session. `copilot --continue` reopens the last one; the docs name `/resume` and `--continue` as the way to the session picker ([docs](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference)). The connector's `resume` is `["--continue"]`.
- `copilot -p "<prompt>"` is the one-shot form; it is not chat mode and is not used.

## 2. Install and record a version

- `npm install -g @github/copilot@<version>` (Node 22 or later), or `curl -fsSL https://gh.io/copilot-install | bash` ([docs](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli)). The package is a Node single-executable app with a Rust addon (package).
- `copilot version` prints the version ([docs](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference)). goalcup's probe runs `copilot --version` instead; whether that form works on 1.0.105 is not measured, and if it fails the `start` event records no version (the run is not stopped). Fifteen stable releases between 30 Sep and 28 Oct (npm); expect more during a run.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `~/.copilot` | `~/.copilot` | unknown |
| Session store | `session-state/<id>/events.jsonl`; `session-store.db` | same | unknown |
| Logs / cache | `logs/process-{timestamp}-{pid}.log`; `~/.cache/copilot/pkg/` holds the unpacked package | same | unknown |
| Env vars that move these | `COPILOT_HOME` | `COPILOT_HOME` | `COPILOT_HOME` |

All from the [config directory reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference), except the package cache, which is from the package (`COPILOT_CACHE_HOME` moves it; the connector does not set it). `settings.json` holds personal settings, `config.json` internal state (and the sign-in token where no keyring exists), `providers.json` the bring-your-own-key registry with keys in plain text. `session-store.db` is a SQLite index the CLI re-creates if deleted.

## 4. Session format and identity

- One folder per session, `session-state/<id>/`, with an append-only `events.jsonl` that supports `--continue` ([docs](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference)). The connector reads every `events.jsonl` below it, and copies `session-store.db*` at the end.
- Subagents have their own subagent session (changelog 1.0.81). Whether that is a separate folder or lines inside the parent's file: unknown.

## 5. Auth

- GitHub sign-in, or bring your own key with no GitHub login: "If you use your own LLM provider API keys (BYOK), GitHub authentication is not required"; without a login `/delegate` and the GitHub MCP server are unavailable ([docs](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli)). Tokens come from `COPILOT_GITHUB_TOKEN`, `GH_TOKEN` or `GITHUB_TOKEN`, in that order.
- Does the harness have a variable that moves its config and login? Yes, `COPILOT_HOME`; the connector sets it.
- `COPILOT_OFFLINE=true` stops contact with GitHub's servers ([docs](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models)). The connector does not set it; that is the participant's choice.

## 6. Network

- Proxy: `HTTPS_PROXY` is first of four variables, "listed from highest to lowest priority" ([docs](https://docs.github.com/en/copilot/concepts/network-settings)). The settings key `proxyUrl` is "overridden by the `HTTP_PROXY` or `HTTPS_PROXY` environment variables" (config reference), so the environment wins and no refusal is needed. HTTP/1.1 by default since 1.0.57 (changelog).
- CA: `NODE_EXTRA_CA_CERTS`, read with the system store ([docs](https://docs.github.com/en/copilot/concepts/network-settings)). Which layer dials, the embedded Node or the Rust addon, is unknown; if the addon reads only its own roots, the run fails TLS loudly, not silently.
- Model hosts: `*.githubcopilot.com` is a vendor host (decrypted and recorded). `github.com` and `api.github.com` carry sign-in and the token exchange but also far more; both pass as tunnels, are not decrypted and are not read.
- DeepSeek needs the `anthropic` provider type, base `https://api.deepseek.com/anthropic`; the `openai` type fails with HTTP 400, "The reasoning_content in the thinking mode must be passed back to the API" ([DeepSeek guide](https://github.com/deepseek-ai/awesome-deepseek-agent/blob/main/docs/copilot_cli.md)). `providers.json` takes `type` of `openai`, `azure` or `anthropic` and a `baseUrl` ([docs](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models)). The connector hints `providers.*.baseUrl` and the variable `COPILOT_PROVIDER_BASE_URL`, both named, so a custom host is recorded.

## 7. Process model

- Subagents inherit the provider configuration (changelog; GitHub's BYOK announcement). Whether they run in the same process: unknown. `/fleet` runs many; the depth and concurrency caps are Copilot's, not ours.
- Exit codes and SIGTERM: unknown.

## 8. Things Goal Cup cannot isolate

- `/delegate` hands work to Copilot cloud agent on GitHub ([docs](https://docs.github.com/en/copilot/how-tos/copilot-cli/use-copilot-cli-agents/delegate-tasks-to-cca)); that work runs on GitHub's machines, with no local trace.
- Session sync to GitHub (`remoteExport`, default `true`, when signed in) and the voice runtime, which ignores proxy settings (issue 5245).
- A sandbox with its own proxy, MCP servers that are not Node and may not trust the CA, and the GitHub MCP server.

## 9. Quirks and open questions

- Auto-update is on by default (`autoUpdate`, [docs](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference)). `COPILOT_AUTO_UPDATE=false` turns it off; that name is read in the package loader and is not on the docs pages read. The connector sets it.
- Unknown: the CA layer, subagent file layout, a BYOK start with no GitHub login and no `gh` (issue 5256 describes a busy loop on Windows).

## Run without permission prompts

`defaultPermissionMode` accepts `"manual"`, `"assisted"` or `"allow-all"` ("broad permissions equivalent to `--allow-all`"); a managed `permissions.defaultMode` overrides it ([docs](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference)). The connector's `no_prompts` row writes `"allow-all"` into `settings.json`.

## Pad result

Not yet run on the pad. Expected: likely verified if the CA layer holds with the DeepSeek key on the `anthropic` type. [`connectors/tiers.toml`](../../connectors/tiers.toml) is the record.
