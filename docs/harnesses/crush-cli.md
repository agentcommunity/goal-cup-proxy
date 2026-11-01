# Crush harness facts

Connector id: `crush-cli` · Facts for version: `0.99.0` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Crush's public README and source at `v0.99.0`. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/crush-cli.toml`](../../connectors/crush-cli.toml))

Source words: "README" is [github.com/charmbracelet/crush](https://github.com/charmbracelet/crush/blob/v0.99.0/README.md) at `v0.99.0`, read on 2026-10-31; "source" is the same tag, read on 29 Oct 2026 and not run; "unknown" is not measured or read. Nothing was run on a real Crush this cycle, so no fact here is "measured".

## 1. Run one prompt, in chat mode first

- `crush` opens the interactive session in the working folder. By default each new invocation starts a fresh session, and the README points to the session manager to go back to one ([README](https://github.com/charmbracelet/crush/blob/v0.99.0/README.md)).
- The connector's `resume` is `["--continue"]`. The README does not document that flag; it is taken from the 0.99.0 source named in the connector header.
- `crush run "<prompt>"` is the one-shot form (source); it is not chat mode and is not used. How to quit: unknown.

## 2. Install and record a version

- `npm install -g @charmland/crush@0.99.0`; its postinstall downloads the matching GitHub release. Release assets, Homebrew, Nix and AUR also work (README, source). About one release a week (npm).
- The connector sets no `version` key; the `start` event records the version that ran.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `./.crushrc`, `./crushrc`, then `~/.config/crush/crushrc` (README); legacy `crush.json` files are still read | same | `%USERPROFILE%\.config\crush\crushrc` (README) |
| Session store | `<workspace>/.crush/crush.db` (source) | same | same |
| Logs / cache | `./.crush/logs/crush.log` (README); state in `~/.local/share/crush/crush.json` | same | state in `%LOCALAPPDATA%\crush\crush.json` (README) |
| Env vars that move these | `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`; `CRUSH_GLOBAL_CONFIG` and `CRUSH_GLOBAL_DATA` override the user config and data (README) | same | unknown |

The connector sets the four XDG folders under `{home}` and unsets `CRUSH_GLOBAL_CONFIG`, `CRUSH_GLOBAL_DATA` and `CRUSH_CLIENT_SERVER`. The database is not in the home: it sits in the workspace, in the data directory `.crush` (`data_directory`, `--data-dir`). Crush uses the closest existing `.crush` between the working folder and the git root (source), so a run started below a repository that already has a `.crush` writes `crush.db` there, not in `{workspace}/.crush`, which is the only place the connector reads.

## 4. Session format and identity

- One SQLite database for all sessions of that folder, with tables `sessions`, `messages` and `files`. A subagent is a child session row with `parent_session_id` (source). The connector copies `crush.db*` as a set when the run ends (`final`).
- The workspace file also holds the participant's earlier Crush sessions for that folder, and the end-of-run copy takes the file whole. The session id form and where it first appears: unknown.

## 5. Auth

- A provider key in the environment, or pasted in the model picker; a pasted key is written as `providers.<id>.api_key` in the data-folder `crush.json`, under `{home}` with the XDG setting (source). Charm's Hyper provider takes `HYPER_API_KEY` (README). The connector lists the data and config `crush.json` files and the workspace ones as secret files.
- Does the harness have a variable that moves its config and login? Two, `CRUSH_GLOBAL_CONFIG` and `CRUSH_GLOBAL_DATA`; the connector uses the XDG folders instead and unsets those two, so the participant's values cannot send the login elsewhere.

## 6. Network

- Proxy: `HTTPS_PROXY` through Go's default transport (source). Go: no `NODE_USE_ENV_PROXY` need.
- CA: no Crush-specific variable. `SSL_CERT_FILE` replaces Go's root list on Linux, so goalcup sets it to the full bundle (source; Go `crypto/x509`). The connector's `ca_env` is empty.
- Hosts: DeepSeek is a native provider, `api.deepseek.com/v1`, type `completions` (Catwalk provider catalog). Crush also calls `catwalk.charm.land` at start (turn off with `CRUSH_DISABLE_PROVIDER_AUTO_UPDATE`), `data.charm.land` for usage metrics (`CRUSH_DISABLE_METRICS` or `DO_NOT_TRACK`) and `api.github.com` for an update notice; these pass as tunnels. `hyper.charm.land` is a gateway host.
- Model address hints: `providers.*.base_url` in `{home}/.config/crush/crush.json`, `{home}/.local/share/crush/crush.json`, `{workspace}/crush.json`, `{workspace}/.crush.json` and `{workspace}/.crush/crush.json`, plus the variable `HYPER_URL`. A provider added in a `crushrc` (a shell script), in a parent folder's config or in `/etc/crush/crush.json` is not hinted, so a call to it stays a tunnel until the participant names the host. Providers are added with `provider add <id> --type <type> --base-url <url>` (README). goalcup never sets a model address.
- Calls use Go's HTTP/2 negotiation, and the proxy offers HTTP/1.1; no WebSocket on the model path (source, unverified by a run).

## 7. Process model

- Crush is one Go process. The `agent` tool starts a subagent in the same process; it has read-only search tools only (glob, grep, ls, view), several can run at once, and each gets its own child session row. A second subagent sits behind `agentic_fetch`, which fetches a web page and has a model summarise it (source). A task that needs edits does not use them.
- With `CRUSH_CLIENT_SERVER=1` the interface talks to a separate `crush server` process (source). Exit codes and SIGTERM: unknown.

## 8. Things Goal Cup cannot isolate

- A `crush server` already running keeps its own environment; the connector unsets `CRUSH_CLIENT_SERVER` for the launched process only.
- Sessions of a `.crush` above the workspace (section 3) are not read from the session file; the wire is still recorded.
- A `crushrc` is a shell script and runs at load; anything it adds is not read for hints.

## 9. Quirks and open questions

- The binary does not update itself; it shows an update notice from `api.github.com` (source). The connector sets no update switch. The provider catalog does refresh at start (section 6).
- Unknown: the Windows XDG behaviour, whether the `crush.json` data file appears with an environment-only key, the session id form.

## Run without permission prompts

Crush asks before tool calls; `--yolo` skips every prompt, and `permissions allow` lines in a `crushrc` allow single tools (README). The connector has no `no_prompts` row: the flag is not a settings key, and a `crushrc` is not a file goalcup writes.

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `docs/plans/2026-10-30-v1.0/PAD-RESULTS.md`; this sheet does not copy them.
