# Kilo CLI harness facts

Connector id: `kilo-cli` · Facts for version: `7.8.25` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Kilo's public docs, the public `Kilo-Org/kilocode` source at `v7.8.25`, and one install of `@kilocode/cli@7.8.25` in a build box. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/kilo-cli.toml`](../../connectors/kilo-cli.toml))

Source words: "docs" is [kilo.ai/docs/code-with-ai/platforms/cli](https://kilo.ai/docs/code-with-ai/platforms/cli) as read on 2026-10-31; "source" is `Kilo-Org/kilocode` at `v7.8.25` (`packages/opencode/src/config/`, `packages/core/`); "measured" is the build-box install of 7.8.25; "unknown" is not measured or read. Kilo ships several builds a day, and on 29 Oct the npm `latest` tag (7.8.18) was behind the `rc` tag (7.8.25). Nothing was run against a model.

## 1. Run one prompt, in chat mode first

- `kilo` opens the TUI in the current folder. `kilo --continue` (`-c`) resumes the most recent session of the workspace; it cannot be combined with `--auto` or a prompt ([docs](https://kilo.ai/docs/code-with-ai/platforms/cli); measured `--help`). The connector's `resume` is `["--continue"]`.
- `kilo run "<prompt>"` is the one-shot form. Without `--auto` it cannot ask and rejects every permission request, exiting 1 (docs). The connector runs the TUI.

## 2. Install and record a version

- `npm install -g @kilocode/cli@7.8.25` (pin; `latest` lags), or a release tarball. The npm `bin/kilo` is a small Node launcher that starts the native Bun binary with the same environment (source).
- `kilo --version` prints `7.8.25` (measured). The connector sets no `version` word, so any build runs and the `start` event records the one that ran.
- Kilo installs patch releases by itself at start unless `KILO_DISABLE_AUTOUPDATE=1` or `"autoupdate": false` in `kilo.json`; a minor or major only shows a notice (source). The connector sets the variable.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$XDG_CONFIG_HOME/kilo/`, default `~/.config/kilo/` (docs; measured with `kilo debug paths`) | same | unknown |
| Session store | `$XDG_DATA_HOME/kilo/kilo.db`, or `kilo-<channel>.db` off the latest channel; `KILO_DB` moves it (source; measured `kilo-rc.db`) | same | unknown |
| Logs / cache | `$XDG_DATA_HOME/kilo/log/`, `~/.cache/kilo/`, `~/.local/state/kilo/`, `/tmp/kilo` (source) | same | unknown |
| Env vars that move these | `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` (measured) | same | unknown |

The connector sets the four XDG variables under the isolated home and leaves `HOME` alone.

## 4. Session format and identity

- One SQLite database for all sessions. The `task` tool starts a child session with `parentID` set, in the same database (source). The connector copies `kilo*.db*` as a set when the run ends (`final`). Table and column names for Kilo, and the session id form: unknown.

## 5. Auth

- A key comes from the environment (`DEEPSEEK_API_KEY` and the like), `kilo auth login` (stored in `$XDG_DATA_HOME/kilo/auth.json`), or an `apiKey` in a config file (source). Kilo's own gateway needs a Kilo account; other providers do not.
- Does the harness have a variable that moves its config and login? No single one; the four XDG variables move config, data, state and cache together, so the connector sets all four and the participant signs in once, in `goalcup setup`. The connector lists `auth.json` and the global config files as secret files.

## 6. Network

- Proxy: `HTTPS_PROXY`. Bun's `fetch` reads it, as measured for the OpenCode fork (OpenCode sheet); not measured for Kilo. The TUI talks to its own local server, so loopback must stay out of the proxy (`NO_PROXY`), as for OpenCode.
- CA: the binary is built with `--use-system-ca`, so it trusts the OS store; `NODE_EXTRA_CA_CERTS` should add to it (source; measured for OpenCode, not Kilo). The effect of `SSL_CERT_FILE` with that flag is unverified.
- Hosts: the provider's own (for example `api.deepseek.com`), Kilo's gateway (`api.kilo.ai`, `ai-gateway.kilo.ai`) or Vercel AI Gateway (`ai-gateway.vercel.sh`) set as a provider's `options.baseURL` ([Vercel's Kilo guide](https://vercel.com/docs/ai-gateway/coding-agents/kilo-code)). `models.dev` (model catalog), the npm registry and `us.i.posthog.com` (telemetry, on by default; `KILO_TELEMETRY_LEVEL=off` stops it) pass as tunnels (source). Remote sessions use a WebSocket, only with a Kilo login (source).
- Model address: `provider.*.options.baseURL` in the config files in section 7; the connector hints them. goalcup never sets it.
- On an unknown model id, Kilo may fall back to a paid gateway model, and the default title model may be paid (shown for OpenCode; unverified for Kilo).

## 7. Config files and which one wins

- Global files, all read and merged in this order, a later file winning on the same key: `config.json`, `kilo.json`, `kilo.jsonc`, `opencode.json`, `opencode.jsonc`, all in `$XDG_CONFIG_HOME/kilo/` (source, `config.ts`). When Kilo writes its global config (first run, `/auto-approve`, "always run") it writes into the first file that exists of `kilo.jsonc`, `kilo.json`, `opencode.jsonc`, `opencode.json`, `config.json`; in an empty home that is `kilo.jsonc` (measured).
- Project files (source, `paths.ts`): `kilo.jsonc`, `kilo.json`, `opencode.jsonc`, `opencode.json` from the launch folder up to the worktree root, then the same names in `.kilocode/` and `.kilo/`. Project settings win over global ones (docs). After all files come `KILO_CONFIG`, `KILO_CONFIG_CONTENT` and managed settings, each later one winning (source).
- Every file is read as JSON with comments, whatever its extension (source). The connector's hint rows read `provider.*.options.baseURL` from the global and project files above and read a `.jsonc` file the same way.

## 8. Things Goal Cup cannot isolate

- `KILO_CONFIG`, `KILO_CONFIG_DIR` and `KILO_CONFIG_CONTENT` in the participant's environment can name a provider `baseURL` no hint reads; its host is then a tunnel, which the per-host flag shows.
- Kilo also reads `.kilo/` and `.kilocode/` in the real home directory (source); goalcup does not move `HOME`, so a `baseURL` there is not hinted either.
- A participant logged in to Kilo sends model calls to Kilo's gateway: recorded, but a different wire.
- Managed and organisation settings: unknown.

## 9. Quirks and open questions

- The `kilo-v2` line on npm may change the session store.
- Open: every "unknown" above; whether `kilo --continue` picks the right session after the XDG switch; whether parallel subagents (`KILO_ENABLE_PARALLEL`, experimental) change the child-session layout.

## Run without permission prompts

`permission` takes one action for every tool, `"allow"`, `"ask"` or `"deny"`, and a global `"*"` rule sets the default (docs). The connector's `no_prompts` row writes `permission = "allow"` into `opencode.json` in the global config folder. That file is merged after `kilo.json` and `kilo.jsonc`, so a `kilo.jsonc` Kilo writes on its first run does not undo it; if `opencode.json` exists first, Kilo writes into it and keeps the key. Only `opencode.jsonc` or a project file can override it, and those are the participant's own choice. See [HARNESS-SETUP](../HARNESS-SETUP.md).

## Pad result

Not yet run on the pad. Tier: not verified: not yet run on the pad. [`connectors/tiers.toml`](../../connectors/tiers.toml) is the record.
