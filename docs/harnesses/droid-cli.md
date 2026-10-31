# Factory Droid harness facts

Connector id: `droid-cli` · Facts for version: `0.251.0` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Factory's public docs and from strings in the shipped `0.251.0` binary. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/droid-cli.toml`](../../connectors/droid-cli.toml))

Source words: "docs" is [docs.factory.com](https://docs.factory.com/cli/configuration/settings) as read on 2026-10-31 (the older `docs.factory.ai` address redirects there); "binary" is a string or code check of the `0.251.0` executable; "unknown" is not read or measured. No Factory account exists this cycle, so Droid was never run through the proxy: everything about its behaviour is read, not measured.

## 1. Run one prompt, in chat mode first

- `droid` with no argument starts the interactive REPL ([docs](https://docs.factory.com/cli/configuration/cli-reference)).
- `droid --resume` (alias `-r`) reopens the last modified session; the connector's `resume` is `["--resume", "--last"]`. Whether `--last` is accepted by `0.251.0` is unknown (the docs show `--resume [sessionId]` only).
- `droid exec` is the non-interactive form. It needs `FACTORY_API_KEY` and is not used.
- With no stored sign-in, Droid opens a browser sign-in. The connector's `first_screen` line tells the participant to sign in before typing a prompt.

## 2. Install and record a version

- `npm install -g droid@<version>` (per-platform binary package, auto-update off at build time), or the standalone installer, which updates itself ([docs](https://docs.factory.com/cli/configuration/cli-reference)). Linux needs `xdg-utils` for the browser sign-in.
- `droid --version` prints the version. The connector sets no `version` word; the `start` event records the build that ran.
- Auto-update: `FACTORY_DROID_AUTO_UPDATE_ENABLED=false`, which must be in the environment of every Droid process, a daemon included (docs). The connector sets it. The settings file has no personal switch (docs).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HOME/.factory/settings.json`, `settings.local.json`; project `.factory/` ([docs](https://docs.factory.com/cli/configuration/settings)) | same | `%USERPROFILE%\.factory\settings.json` |
| Session store | `$HOME/.factory/sessions/<slug>/<uuid>.jsonl` and `<uuid>.settings.json` (binary) | same | unknown |
| Logs / cache | `$HOME/.factory/logs/`, `certs`, `updates` (binary) | same | unknown |
| Env vars that move these | `FACTORY_HOME_OVERRIDE` (binary) | same | unknown |

`FACTORY_HOME_OVERRIDE` is not in the docs. In the `0.251.0` binary it replaces the `.factory` folder's parent and nothing else, so `HOME` stays the participant's and goalcup writes no `.profile` for Droid. A legacy `~/.factory/config.json` (snake_case `custom_models`, `base_url`) is copied into `settings.json` at start and then ignored (docs); the connector reads both for hints.

## 4. Session format and identity

- JSONL, one file per session, beside a `<uuid>.settings.json` that is rewritten as the session changes (binary). The connector copies the settings file as a `snapshot` and the transcript as `sessions`. Side chats sit under `sessions/<slug>/btw/` (binary).
- The session id is the file name. Where it first appears on screen: unknown.
- Subagent file layout: unknown. Subagents run through the `Task` tool as built-in droids (`worker`, `explorer`) or custom ones in `.factory/droids/` ([docs](https://docs.factory.com/harness/subagents)); whether each writes its own file is not read.

## 5. Auth

- A Factory account is always required, even with a bring-your-own key: browser sign-in in the REPL, or `FACTORY_API_KEY` for `droid exec` (docs). The sign-in token goes to the OS keyring, with a file fallback when `FACTORY_DISABLE_KEYRING` is set (binary, unverified). The connector lists `auth.json` as a secret file.
- Model key: `customModels[]` in `settings.json` with `model`, `displayName`, `baseUrl`, `apiKey`, `provider` (`anthropic`, `openai`, `generic-chat-completion-api`, `bedrock-converse`) ([docs](https://docs.factory.com/cli/byok/overview)). `apiKey` accepts `${VAR}`; use that, so no key is written to disk. DeepSeek is an OpenAI-compatible `baseUrl` with `generic-chat-completion-api` (no docs example).
- A variable that moves config and login together: yes, `FACTORY_HOME_OVERRIDE` (binary).

## 6. Network

- CA: `NODE_EXTRA_CA_CERTS` or `FACTORY_EXTRA_CA_CERTS` ([docs](https://docs.factory.com/enterprise/network-and-deployment)). At start Droid merges bundled, system and extra roots, so both add. goalcup sets `NODE_EXTRA_CA_CERTS` for every harness, and the connector's `ca_env` points `FACTORY_EXTRA_CA_CERTS` at the run CA too.
- Proxy: `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`; the binary says OS proxy and PAC settings are ignored (docs; binary). Bun runtime. Unmeasured.
- Where the model host is named: `customModels.*.baseUrl` in the global and workspace settings files, and `custom_models.*.base_url` in `config.json`. Hint variables: `FACTORY_API_BASE_URL`, `FACTORY_API_BASE_URL_EU`. A `${VAR}` base URL is not resolved.
- `api.factory.ai` and `api.eu.factory.ai` (auth, org settings, telemetry, cloud session sync) are listed as vendor hosts and recorded. Cloud session sync defaults on (`cloudSessionSync: true`, docs), so the transcript also goes to Factory. Sign-in token refresh goes to `api.workos.com`, which is on the never-decrypt list and passes through as a tunnel.
- Not read: whether HTTP/2 or WebSocket carries model calls (WebSocket is used by the daemon relay, binary).

## 7. Process model

- The REPL talks to a background daemon. When Droid starts it as its own child it inherits the launch environment (binary, unverified).
- `droid daemon` has no `stop` subcommand ([docs](https://docs.factory.com/cli/configuration/cli-reference)), so the connector has no `before` or `after`. A daemon or the Factory desktop app started earlier keeps its own environment and calls outside the proxy.
- Exit codes and SIGTERM: unknown.

## 8. Things Goal Cup cannot isolate

- A separately started `droid daemon` or the desktop app, and cloud Droid Computers, which run on Factory's side.
- Org-managed settings (`maxAutonomyLevel`, `disableAutoUpdate`) merged above the user's.
- Subagents with BYOK are reported broken ([issue 691](https://github.com/Factory-AI/factory/issues/691): subagent returns nothing or a 402 and the parent does the work). Not confirmed on `0.251.0`; point 2 may be untestable with a bring-your-own key.

## 9. Quirks and open questions

- Unknown: subagent file layout, the Windows session paths, whether `--resume --last` is valid, and whether a second Droid already running holds the daemon for the whole account.
- No Factory account was available this cycle, so no run exists.

## Run without permission prompts

`sessionDefaultSettings.autonomyLevel` in `settings.json`: `off` (default, manual approval), `low`, `medium`, `high` ([docs](https://docs.factory.com/cli/configuration/settings)). The connector's `no_prompts` row writes `high` into the isolated home. An org cap (`maxAutonomyLevel`) can lower it. See [HARNESS-SETUP](../HARNESS-SETUP.md).

## Pad result

Not yet run on the pad. The tier and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml).
